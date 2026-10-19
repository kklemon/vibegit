# Refactor the Git layer

Status: implemented by the Rust rewrite (0.3.0, 2026-10-19). The architecture below
maps onto `src/git/`, `src/changes.rs` and `src/workflow.rs`; the scenario table became
`tests/git_edge_cases.rs`. Kept as the design rationale.

Every "verified" claim below was reproduced with a scratch harness that builds a
throwaway repository per scenario and runs the real `vibegit.git` pipeline
(`get_git_status` → `GitContextFormatter.format_changes` → `stage_commit_proposal`
→ `commit_commit_proposal`) against it, with the user's global git config disabled.
The harness is a good starting point for the regression suite in phase 0.

## Goals
- Make `vibegit commit` correct for the repository states git users actually have:
  submodules, renames, mode changes, binaries, CRLF, detached or unborn HEAD,
  in-progress merges, conflicts, nested repositories, non-default `diff.*` config.
- Guarantee one invariant end to end: **VibeGit never modifies the working tree.
  It only writes the index and creates commits, and every commit contains exactly
  the bytes that are in the working tree.**
- Separate git I/O, patch manipulation, change bookkeeping, workflow, and terminal UI
  so each can be unit-tested without a model and without an interactive terminal.
- Remove the two libraries whose abstractions caused most of the defects (GitPython,
  unidiff) in favour of a small, explicit wrapper around the `git` binary.

## Non-goals
- Changing the LLM prompt strategy, the proposal schemas, or the config file format.
- Supporting a pre-staged index as an input (VibeGit keeps asking to unstage first).
- Windows support beyond not making it worse (see open questions).

## Verified behaviour of the current implementation

### Silent wrong results (worst category)

| Scenario | What happens today | Cause |
| --- | --- | --- |
| `apply.whitespace=error` in the user's config, or any first `git apply` failure | The retry with `--whitespace=fix` succeeds and commits content that differs from the working tree. `git status` afterwards still shows the file as modified. | `stage_commit_proposal` falls back to `--whitespace=fix`, which rewrites the patch. |
| `git merge --no-commit` in progress, user accepts "unstage all" | `git reset HEAD --` deletes `MERGE_HEAD`. The merge is silently aborted, the merged files become untracked, and VibeGit commits them as ordinary new files. History never records the merge. | `reset_staged_changes` uses a full mixed reset; a full reset clears merge state. A pathspec-limited reset (`git reset -q -- :/`) keeps it. |
| Submodule with modified tracked files inside, pointer unchanged | `git diff HEAD -- sub` emits `+Subproject commit <sha>-dirty`. `git apply --cached` prints "Applied patch sub cleanly" but stages nothing. `git commit` then fails with "no changes added to commit". This is the submodule error reported by users. | Dirty-submodule markers are not applicable patches. `--ignore-submodules=dirty` suppresses them. |
| Submodule pointer moved, user has `diff.submodule=log` or `=diff` | `log`: AssertionError while formatting. `diff`: `git apply` fails with "appears as both a file and as a directory". | Porcelain `git diff` output depends on user config. |
| Staged rename (only reachable through `get_git_status` directly) | Only the new path is reported; the deletion of the old path is lost. | GitPython's `index.diff("HEAD")` returns **reversed** entries: a staged add is reported as `change_type='D'`, a staged delete as `'A'`, and for renames `a_path`/`b_path` are swapped. The comment in `get_git_status` describing `a_path`/`b_path` is wrong. The function "supports" staged changes that `stage_commit_proposal` can never apply (verified: three staged scenarios fail with "already exists in index" / "patch does not apply"). |
| Interactive mode → "Apply all remaining" | Commits via GitPython's in-process `index.commit`: no watermark, no `commit.gpgsign`, different hook semantics from the `git commit` used by Yolo mode. Interactive single commits (editor path) never get the watermark either. | Three separate apply loops in `cli.py` with divergent behaviour. |
| Pre-commit hook rejects a commit in interactive mode | Reported as "Error staging changes for proposal". | `GitCommandError` from `git commit` is caught by the staging handler. |

### Crashes

| Scenario | Failure | Cause |
| --- | --- | --- |
| File with CRLF line endings | `git apply`: "patch does not apply" | GitPython strips the trailing `\n` of command output, leaving the last diff line as `c\r`. `stage_commit_proposal` then calls `.strip()`, which also removes the `\r`, corrupting the last context line. |
| `diff.noprefix=true` | `git apply`: "git diff header lacks filename information" | Patch has no `a/` `b/` prefixes; `git apply` defaults to `-p1`. |
| `diff.external` set | AssertionError (empty message) | Unparseable diff output. Plumbing commands ignore this setting. |
| Type change (file replaced by symlink) | AssertionError | `git diff` emits two `diff --git` sections for one path; `assert len(patch_set) == 1`. |
| Untracked nested git repository | AssertionError | `repo.untracked_files` yields `nested/`; `git diff --no-index` fails; `with_exceptions=False` hides the error and the empty output trips the assert. |
| Detached HEAD | TypeError from `repo.active_branch` | No branch to read. |
| Unborn HEAD (no commits) | `BadName: Ref 'HEAD' did not resolve` | `index.diff("HEAD")`. README documents this as unsupported; it does not need to be. |
| Merge conflict present | `git apply`: "does not exist in index" | Unmerged paths are listed as ordinary changed files; no preflight check. |

### Works today and must keep working
Hunk splitting across commits in any order (git's context search finds shifted hunks,
including with repeated identical code blocks), untracked files in new directories,
empty files, deletions, unstaged renames (committed as a rename by git), mode-only
and mode+content changes (even split across commits), binary add/modify/delete,
`-diff` attributes, no newline at EOF (including split hunks), trailing whitespace
under default config, names with spaces, unicode, `[`, `*`, symlinks, intent-to-add
entries, ignored files, `core.quotePath=false`, linked worktrees, running from a
subdirectory, form feed and U+2028 inside content, content lines that look like diff
headers.

## Root causes

1. **Porcelain output is parsed by assumption.** `git diff` and `git diff --no-index`
   honour `diff.noprefix`, `diff.mnemonicPrefix`, `diff.external`, `diff.submodule`,
   textconv drivers and colour settings. The plumbing command `git diff-index -p` is
   immune to all of these (verified under `noprefix`, `external`, `mnemonicPrefix`,
   `submodule=log|diff`).
2. **GitPython abstractions do not match git.** Reversed `index.diff(other)` entries,
   trailing-newline stripping of binary-sensitive output, `untracked_files` returning
   directories, and `index.commit` reimplementing commit in Python.
3. **Patches are reconstructed instead of sliced.** The diff text is parsed into
   unidiff objects, mutated (`hunk.section_header = "# Change ID: n"` leaks into the
   patch sent to `git apply`), deep-copied, re-serialised, then `.strip()`ped and
   padded with `"\n\n\n"`, then applied with a `--whitespace=fix` retry. Each step is a
   workaround for a round-trip problem introduced by the previous one.
4. **No repository preflight.** Nothing checks for unmerged paths, in-progress
   merge/rebase/cherry-pick, detached or unborn HEAD, dirty submodules, or nested
   repositories before the first git write.
5. **Mixed responsibilities.** `CommitProposalContext` owns change IDs, proposal
   validation, patch building, staging, committing and the watermark. `cli.py` runs
   git commands directly, prints, calls `sys.exit` inside library-like code, and has
   three copies of the apply loop. `config.py` imports the formatter class.
6. **Untestable git layer.** `tests/test_git.py` builds `GitStatusSummary(repo=None)`
   and stuffs `None` into `change_id_to_ref` to get a context object at all. The only
   end-to-end test is the paid live run.

## Target architecture

```
vibegit/
  git/
    runner.py    run_git(): subprocess wrapper, bytes in/out, GitError(cmd, code, stderr)
    repo.py      GitRepo: discovery, state, status, diffs, apply, unstage, commit, log, push
    status.py    porcelain v2 parser -> StatusEntry
    patch.py     diff text parser -> FilePatch/Hunk; build_patch() (pure, bytes only)
  changes.py     Change, ChangeSet: numbering, LLM rendering, proposal validation,
                 per-proposal patch assembly with hunk offset bookkeeping (pure)
  workflow.py    CommitWorkflow: preflight -> collect -> propose -> apply; no printing,
                 no sys.exit; typed exceptions and result objects
  cli.py         prompts, tables, pager, exit codes only; no git calls
```

`config.py` becomes plain data (the `get_context_formatter` method moves to
`workflow.py`). `ai.py`, `llm.py`, `prompts.py`, `schemas.py`, `wizard.py` are
untouched except for the Change ID marker format in `prompts.py` (see rendering).

### Invariants the code must enforce
- The working tree is never written. The only git write commands are
  `git apply --cached`, `git reset -q -- <paths>`, `git commit`, `git push`, and
  `git update-index --refresh` (stat cache only).
- Before applying each proposal: index == HEAD, checked with
  `git diff-index --cached --quiet HEAD` (or the empty tree when HEAD is unborn).
  If it fails, stop with a message instead of stacking changes.
- One `git apply --cached` per proposal. `git apply` checks every file before
  touching the index, so a proposal either stages completely or not at all.
- Patch bytes handed to `git apply` are verbatim slices of git's own output. Nothing
  is re-serialised. Only hunk header numbers are rewritten, by the formula below.
- All git output is `bytes`. Text is produced only for display and for the model,
  decoded with `errors="surrogateescape"`.

### Git commands (exact)

Discovery and state:
```
git rev-parse --show-toplevel --git-dir --absolute-git-dir
git rev-parse -q --verify HEAD                    # non-zero => unborn
git symbolic-ref -q --short HEAD                  # non-zero => detached
git rev-parse --git-path MERGE_HEAD --git-path CHERRY_PICK_HEAD --git-path REVERT_HEAD \
    --git-path BISECT_LOG --git-path rebase-merge --git-path rebase-apply
                                                   # one --git-path per name; then os.path.exists on each
git hash-object -t tree /dev/null                  # empty tree id, used instead of HEAD when unborn
```

Status (one call; detects staged entries, unmerged entries, renames, submodule
state, untracked files, nested repos):
```
git -c core.quotePath=false status --porcelain=v2 -z --untracked-files=all
```
Porcelain v2 lines start with `1` (ordinary), `2` (rename/copy), `u` (unmerged), `?`
(untracked), `!` (ignored). Field 3 of `1`/`2` lines is the submodule state
`S<C><M><U>` (`C` = commit changed, `M` = tracked files modified inside, `U` =
untracked files inside). Untracked entries whose path ends with `/` are nested
repositories.

Tracked changes (one call, regardless of file count):
```
git update-index -q --refresh --ignore-submodules
git -c core.quotePath=false diff-index -p --binary --no-ext-diff --no-textconv \
    --no-renames --ignore-submodules=dirty HEAD --
```
`diff-index` is plumbing: it ignores `diff.noprefix`, `diff.mnemonicPrefix`,
`diff.external`, `diff.submodule`, colour and pager settings (verified). The two
explicit `--no-*` flags cover per-driver `diff.<name>.command` and textconv, which
plumbing does honour. `--ignore-submodules=dirty` removes the `-dirty` lines while
still reporting moved pointers (verified). `git apply --cached` stages a
`Subproject commit` hunk correctly (verified).

Untracked files (one call per file; nested repositories are skipped with a warning):
```
git -c core.quotePath=false diff --no-index --no-ext-diff --no-textconv --no-color \
    --binary --src-prefix=a/ --dst-prefix=b/ -- /dev/null <path>
```
Exit code 1 means "differences" and is the normal case; an empty file also exits 1
with a header-only diff. A missing or unreadable file exits 1 too, with empty
stdout and a message on stderr (verified), so success is "exit 0 or 1 and non-empty
stdout"; everything else must be raised, not swallowed as today.

Recent history and branch context:
```
git log -n <N> -z --format=%B                      # skipped when HEAD is unborn
```

Staging, unstaging, committing, pushing:
```
git apply --cached --whitespace=nowarn -           # patch on stdin
git reset -q -- <paths of the failed proposal>     # pathspec-limited: keeps MERGE_HEAD
git commit -m <message>                            # Yolo; stdio inherited so hooks and gpg can talk
git commit -e -m <message>                         # interactive; same function, edit=True
git push                                           # stdio inherited; refuse when detached
```
`--whitespace=nowarn` overrides `apply.whitespace` so the user's setting can neither
reject nor rewrite the patch. The `--whitespace=fix` retry is deleted. Both commit
paths go through one function that applies the watermark and sanitises the message
(strip trailing whitespace, collapse blank runs, prefix lines starting with `#` with
a space so `--cleanup=strip` in editor mode cannot eat them).

### Types

```python
# git/status.py
@dataclass(frozen=True)
class StatusEntry:
    path: str
    orig_path: str | None            # renames/copies
    index_state: str                 # one of ".AMDRCT"
    worktree_state: str              # one of ".MDT"
    kind: Literal["ordinary", "rename", "unmerged", "untracked", "nested_repo"]
    submodule: SubmoduleState | None # commit_changed / tracked_modified / has_untracked

# git/patch.py
@dataclass(frozen=True)
class Hunk:
    old_start: int; old_count: int; new_start: int; new_count: int
    added: int; removed: int
    text: bytes      # "@@ ..." line through the last body line, incl. "\ No newline" markers

@dataclass(frozen=True)
class FilePatch:
    path: str                        # new path, or old path for deletions
    old_path: str | None
    kind: Literal["modified", "added", "deleted", "typechange", "mode", "binary", "submodule"]
    header: bytes                    # "diff --git" through the "+++" line
    hunks: tuple[Hunk, ...]
    raw: bytes                       # every section for this path, verbatim (two for typechange)

    @property
    def atomic(self) -> bool:        # True => the whole file is one change unit
        return self.kind != "modified" or len(self.hunks) <= 1

def parse_patch(data: bytes) -> list[FilePatch]
def build_patch(file: FilePatch, hunks: Sequence[Hunk] | None, *, applied: Sequence[Hunk]) -> bytes
```

`parse_patch` is a line scanner, not a regex grammar: a section starts at
`diff --git `, the header ends at the first line starting with `@@`, `GIT binary
patch`, `Binary files`, or the next `diff --git `. Consecutive sections with the
same path (type changes) are merged into one `FilePatch`. Mode lines, `index` lines,
`rename from/to`, `new file mode`, `deleted file mode` are kept in `header` verbatim.

`build_patch` for an atomic file returns `raw`. For a hunk subset it returns
`header + hunks in file order`, with two adjustments:
- mode lines (`old mode`/`new mode`) are kept only when `applied` is empty, so a mode
  change travels with the first commit that touches the file and is not repeated
  (today the repeat only works because `git apply` downgrades it to a warning);
- hunk headers are recomputed. With `applied` = hunks of this file already committed
  by earlier proposals and `selected` = hunks in this patch, both in file order:

  ```
  old_start' = old_start + Σ (h.new_count - h.old_count) for h in applied  if h.old_start < this.old_start
  new_start' = old_start' + Σ (h.new_count - h.old_count) for h in selected if h.old_start < this.old_start
  ```
  Hunks never overlap (git merges hunks closer than twice the context), so the
  formula is exact. `git apply` would usually find the hunk anyway by searching
  outward from the stated line, but exact positions remove the only ambiguity
  (identical blocks within the shift distance) and the "applied with offset" noise.

```python
# changes.py
@dataclass(frozen=True)
class Change:
    id: int
    file: FilePatch
    hunk: Hunk | None                # None => whole file

class ChangeSet:
    changes: list[Change]
    warnings: list[str]              # skipped nested repos, dirty submodules, oversize files
    def render_for_model(self, truncate_lines: int | None) -> str
    def validate(self, result: CommitProposalsResultSchema) -> None
    def patch_for(self, change_ids: Sequence[int]) -> bytes      # uses self.applied for offsets
    def mark_applied(self, change_ids: Sequence[int]) -> None
```

Rendering for the model is a pure function of the parsed data and no longer mutates
anything. Each file gets a one-line summary (`modified`, `new file`, `deleted`,
`renamed from X`, `mode 100644 -> 100755`, `binary`, `type change`,
`submodule <old7> -> <new7>` followed by `git -C <sub> log --oneline old..new`), and
each change unit is preceded by its own line `# Change ID: <n>`. That matches the
example already in `prompts.py`; today the marker is injected into the `@@` header,
which the prompt does not describe.

`validate` keeps the current coverage/duplication checks and adds: no empty
`change_ids`, no unknown ids, non-empty commit message after sanitising.

```python
# workflow.py
class RepositoryStateError(Exception): ...   # unborn/detached are not errors
class StagingError(Exception): proposal, stderr
class CommitError(Exception): proposal, returncode

@dataclass
class Preflight:
    state: RepoState                         # root, branch | None, head | None, in_progress | None
    staged: list[StatusEntry]
    unmerged: list[StatusEntry]
    warnings: list[str]

class CommitWorkflow:
    def preflight(self) -> Preflight
    def unstage_all(self) -> None                       # git reset -q -- :/ ; refuses if in_progress
    def collect(self) -> ChangeSet
    def propose(self, changes: ChangeSet, instruction: str | None) -> ProposalResult
    def apply(self, changes: ChangeSet, proposal: CommitProposalSchema, *, edit: bool) -> None
    def apply_many(self, changes, proposals, *, edit: bool, on_progress) -> AppliedReport
```

`apply` does, in order: assert index == HEAD; `patch_for`; `git apply --cached`;
`git commit`; on commit failure `git reset -q -- <paths>` and raise `CommitError`;
on success `mark_applied`. Yolo, Yolo-and-push and interactive "apply all" all call
`apply_many`; the interactive single-commit path calls `apply(edit=True)`. This
removes the three divergent loops and gives every commit the same watermark, hooks,
signing and message handling.

### Edge-case policy

| Situation | Target behaviour |
| --- | --- |
| Unborn HEAD | Supported. No tracked diff; untracked files as usual; the first commit becomes the root commit. README note becomes obsolete. |
| Detached HEAD | Supported. Context line reads `Detached HEAD at <short sha>`. Push option hidden. |
| Merge, rebase, cherry-pick, revert or bisect in progress | Refuse before any write, with the git command that resolves it. Never reset in this state. |
| Unmerged paths | Refuse. |
| Staged changes | Keep today's prompt. Unstaging uses the pathspec-limited reset. |
| Submodule pointer moved | One atomic change, rendered with the submodule's new commit subjects. Staged via the plumbing `Subproject commit` hunk. |
| Submodule with modifications inside | Invisible to the parent diff; a warning tells the user to commit inside the submodule first. |
| Nested repository (untracked dir with `.git`) | Skipped with a warning. Git itself cannot add it without `submodule add`. |
| Type change, mode-only, binary, renames in the status | Atomic change units. |
| CRLF, missing final newline, form feed, U+2028, non-UTF-8 content | Bytes end to end; nothing is stripped or re-encoded. |
| Non-ASCII paths | `core.quotePath=false` so headers carry raw UTF-8 (git apply accepts them; verified). Paths with control characters stay quoted and are displayed as git prints them. |
| Oversize untracked files (config `max_change_size_kb`, default 512) | Listed as an atomic change with a size placeholder instead of content, so one generated file cannot blow the prompt. |
| User `diff.*` config, textconv drivers, external diff, pagers, colour | Irrelevant, by construction. |
| `apply.whitespace` config | Overridden with `--whitespace=nowarn`. |
| Hooks that reject | Reported as a commit failure with the hook's output; the proposal's paths are unstaged; remaining proposals are not applied. |
| Hooks that rewrite files (formatters) | The commit succeeds; the formatter's edits show up as leftover changes in the final summary. Not an error. |

## Detailed plan

### Phase 0: lock in behaviour
1. Turn the scratch harness into `tests/git/test_edge_cases.py`: one parametrised
   test per scenario above, each building a temp repo, running collect → apply with a
   deterministic fake model (every change its own commit, in forward and reverse
   order), and asserting `git status --porcelain --untracked-files=all` is empty and
   `git diff HEAD --exit-code` passes.
2. `conftest.py` sets `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`, a local
   `user.name`/`user.email`, `commit.gpgsign=false`, and `protocol.file.allow=always`
   for submodule fixtures.
3. Mark the currently failing scenarios `xfail(strict=True)` so each later phase
   flips them to passing deliberately.

### Phase 1: runner, repo, status, preflight
4. Add `git/runner.py` and `git/repo.py` as described. `GitRepo.discover(cwd)`
   replaces `git.Repo(..., search_parent_directories=True)`.
5. Add `git/status.py` with the porcelain v2 parser and fixture tests
   (ordinary, rename, unmerged, untracked, nested repo, all submodule flag
   combinations, paths with spaces and non-ASCII characters).
6. Add `workflow.preflight()` and wire `cli.py` to it. Detached and unborn HEAD now
   pass; merges in progress and conflicts are refused with a message.

### Phase 2: patches and change set
7. Add `git/patch.py` with `parse_patch`/`build_patch` and fixture tests using diffs
   captured from git for: text with several hunks, CRLF, no final newline, new file,
   deleted file, mode only, mode plus content, binary, type change, submodule, rename
   header, quoted path, empty file, `Binary files differ` (from `-diff` attribute).
8. Add `changes.py` (`ChangeSet`), port `validate_commit_proposal`, add the offset
   formula with a unit test that applies hunks in every permutation of a 4-hunk file.
9. Switch `collect()` to the plumbing diff and `--no-index` commands; switch
   `apply()` to `patch_for` on stdin with `--whitespace=nowarn`. Delete the
   `--whitespace=fix` retry, the temp file, and the `.strip() + "\n\n\n"` padding.
   Update the Change ID marker in `prompts.py` to the standalone-line form.
10. Flip the CRLF, noprefix, external-diff, type-change, nested-repo, submodule and
    conflict tests from xfail to pass.

### Phase 3: workflow and CLI
11. Move `has_staged_changes`, `reset_staged_changes`, `open_editor_for_commit`,
    `get_project_instructions`, and the three apply loops out of `cli.py` into
    `workflow.py` as described. `cli.py` keeps `InteractiveCLI` for prompts and
    output only, with the recursion in `prompt_main_workflow`/`run_commit_workflow`
    replaced by a loop so the final summary prints once.
12. Watermark and message sanitising apply to every commit path.
13. Smaller, unrelated cleanups that are cheap while the file is open:
    `check_for_update()` runs at import time (network call on every invocation,
    including `--help` and tests) and should move into the `commit` command behind a
    daily cache; `config open` uses the macOS `open` binary and should use
    `click.launch`; the pager should use `click.echo_via_pager` instead of spawning
    `less` on a temp file.

### Phase 4: dependencies and docs
14. Remove `gitpython` and `unidiff` from `pyproject.toml` and the lock file. The
    remaining GitPython uses and their replacements:

    | GitPython use | Replacement |
    | --- | --- |
    | `git.Repo(cwd, search_parent_directories=True)` | `git rev-parse --show-toplevel` |
    | `repo.index.diff("HEAD")`, `repo.index.diff(None)`, `repo.untracked_files` | porcelain v2 status |
    | `repo.git.diff(...)` | `git diff-index -p`, `git diff --no-index` (bytes) |
    | `repo.git.execute(["git", "apply", ...])` | `run_git(["apply", "--cached", ...], input=patch)` |
    | `repo.git.commit("-m", ...)`, `repo.index.commit(...)` | one `GitRepo.commit(message, edit)` |
    | `repo.active_branch.name` | `git symbolic-ref -q --short HEAD` |
    | `repo.iter_commits()` | `git log -n N -z --format=%B` |
    | `origin.push(branch)` + `PushInfo` flags | `git push` with inherited stdio, exit code |
    | `repo.git.reset("HEAD", "--")` | `git reset -q -- :/` or `-- <paths>` |

    unidiff passed every byte round-trip test in the harness; it is removed for
    design reasons (verbatim slicing makes fidelity a property of the code rather
    than of a dependency, and the mutation/deepcopy/re-serialise dance goes away),
    not because of an observed bug.
15. README: drop the "requires at least one commit" note, add a short "What VibeGit
    does to your repository" section stating the working-tree invariant, and
    document the submodule and nested-repository policy.
16. STATE.md: record the refactor.

## Testing strategy summary
- Pure unit tests with captured fixtures for `status.py`, `patch.py`, `changes.py`
  (fast, no git binary needed except to regenerate fixtures).
- Temp-repo integration tests for `GitRepo` and `CommitWorkflow` covering every row
  of the two tables in "Verified behaviour", run in CI (git is already available).
- The existing live pipeline test stays opt-in and unchanged in intent.

## Risks and open questions
- **Windows.** `git diff --no-index -- /dev/null <path>` is the standard idiom and
  git special-cases the literal `/dev/null`, but this has not been verified on
  Windows in this review. `click.launch` and inherited-stdio commits are
  cross-platform. If Windows matters, add a CI job.
- **Rename detection.** After unstaging, a rename is a deletion plus an untracked
  file, which the model sees as two units and usually groups correctly (git then
  records a rename). Detecting renames before the model sees them would require a
  temporary index (`GIT_INDEX_FILE=... git add -A && git diff-index -M --cached`),
  which writes blob objects for every untracked file. Deferred.
- **Oversize files.** The default cap is a guess; make it configurable and report
  skipped content in the warnings list.
- **Hooks that need a terminal.** Both commit paths inherit stdio, so `gpg`
  pinentry and interactive hooks work in Yolo mode too. This changes today's Yolo
  behaviour, where output is captured and shown only on failure.
