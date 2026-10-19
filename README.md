<h1 align="center">VibeGit</h1>

<p align="center">
  <i>Turn a busy Git working tree into focused, reviewable commits.</i>
</p>

<p align="center">
  <a href="https://pypi.org/project/vibegit/" target="_blank">
    <img alt="PyPI - Version" src="https://img.shields.io/pypi/v/vibegit">
  </a>
  <a href="https://github.com/kklemon/vibegit/actions/workflows/ci.yml" target="_blank">
    <img src="https://github.com/kklemon/vibegit/actions/workflows/ci.yml/badge.svg" alt="CI status">
  </a>
</p>

---

<p align="center">
    <img src="resources/before-vibegit.png" alt="Working tree before using VibeGit" width="45%">
    <img src="resources/after-vibegit.png" alt="Working tree after using VibeGit" width="45%">
</p>

<p align="center">
  <i>An example working tree before and after VibeGit groups the changes.</i>
</p>

---

## Turn mixed changes into focused commits

During a long coding session, unrelated changes pile up in the same working tree. Turning them into small, coherent commits means inspecting hunks, staging them one by one and writing matching messages.

Run VibeGit from anywhere inside the repository:

```bash
vibegit commit
```

VibeGit reads the diff, the current branch and the recent history, asks a model for semantically related groups of hunks with commit messages, and opens an interactive interface where you review, rename, reorder, skip and apply them.

VibeGit is a single native binary (written in Rust) that drives the `git` command line. It never modifies your working tree: it only stages exactly the hunks it was told to and creates commits, so every commit contains precisely the bytes that are in your files.

## Features

* **Semantic grouping:** related hunks become one commit, including changes that span several files.
* **Generated messages:** a concise subject (and body, if the project style uses one) for every group.
* **Interactive review:** scroll through proposals, read the diff, edit messages in place or in `$EDITOR`, skip or reorder proposals, commit one at a time or all at once, push when done.
* **Project rules and one-off instructions:** `.vibegitrules` steers message style and granularity; `-i "..."` overrides it for one run and can also be entered from the interface.
* **Change exclusions:** the model may set aside changes that look unfinished or sensitive; they are shown separately and never committed silently.
* **Many providers:** Google Gemini, OpenAI, xAI Grok, Anthropic, Ollama, Groq, DeepSeek, Cohere, OpenRouter, Together, Fireworks, and any OpenAI-compatible endpoint.
* **Robust git handling:** binaries, mode changes, symlinks, type changes, submodule pointer updates, CRLF, missing final newlines, unicode paths, detached and unborn HEAD, and unusual `diff.*` configuration all work; merges in progress and conflicts are refused before anything is written.

## Installation

### With uv (recommended)

```bash
uv tool install vibegit
```

Prebuilt wheels contain the binary, so no Rust toolchain or Python dependencies are needed. Upgrade with `uv tool upgrade vibegit`, or try it without installing:

```bash
uvx vibegit --help
```

`pipx install vibegit` and `pip install vibegit` work the same way.

### From source

```bash
cargo install --path .
```

Git 2.25 or newer must be on your `PATH`.

## Quick start

```bash
vibegit init     # add a starter .vibegitrules to the project (optional)
vibegit config   # pick a model and store its API key
vibegit commit   # review and apply proposed commits
```

The setup wizard also runs automatically the first time `vibegit commit` finds no configuration.

## The interface

```
 VibeGit  ~/projects/app on main  model google:gemini-3.8-flash
┌ Proposals (3 pending, 1 committed) ─┐┌ Proposal 2 of 4 ───────────────────────────┐
│ ✔  1 add rust git runner            ││ Message                                    │
│▶○  2 parse porcelain v2 status      ││   parse porcelain v2 status                │
│ ○  3 wire the interface             ││                                            │
│ ⊘  4 fix typo in readme             ││ Explanation                                │
│ !  excluded: scratch.py             ││   These hunks introduce the status parser  │
│                                     ││                                            │
│                                     ││ Files (2)                                  │
│                                     ││   M src/git/status.rs +120 -3              │
│                                     ││   A tests/status.rs +80 -0                 │
└─────────────────────────────────────┘└────────────────────────────────────────────┘
 4 proposal(s) for 9 change unit(s) in 5 file(s).
 [c Commit] [C Commit all] [e Edit] [d Diff] [s Skip] [i Instruct] [r Regenerate] [p Push] [? Help] [q Quit]
```

The bottom row is a button bar. Every button shows its shortcut; `Tab` moves the keyboard focus into the bar, `←` `→` choose a button, `Enter` runs it, and buttons that do not apply to the selection are dimmed. The mouse works too: click a button or a proposal, scroll with the wheel. Dialogs show their choices as buttons as well.

| Key | Action |
| --- | --- |
| `↑` `↓` `j` `k`, `PgUp` `PgDn`, `g` `G` | Move through the proposals |
| `Enter` or `d` | Show the full diff of the selection (`n`/`p` jump between proposals) |
| `c` | Stage and commit the selected proposal |
| `C` or `a` | Commit all pending proposals in list order |
| `e` | Edit the commit message in place (`Ctrl+S` saves, `Esc` cancels) |
| `E` | Edit the commit message in `$EDITOR` |
| `s` | Skip or unskip a proposal |
| `J` `K` or `Alt+↓` `Alt+↑` | Move the proposal down or up |
| `[` `]` | Scroll the detail pane |
| `i` | Regenerate the proposals with a one-off instruction |
| `r` | Regenerate the proposals |
| `p` | Push the created commits (sets an upstream when needed) |
| `m` or `Space` | Open the action menu |
| `?` | Show all keys |
| `Tab` `Shift+Tab` | Focus the button bar (`←` `→` choose, `Enter` runs, `Esc` leaves) |
| `q` | Quit; a summary of the created commits is shown first |
| `Y` (in any confirmation) | Confirm and never ask again for that action; see `confirmations.*` below |

Excluded groups appear at the end of the list in yellow. They are never committed; regenerate with an instruction (`i`) if they should be.

## Non-interactive use

```bash
vibegit commit --yolo            # apply every proposal, no interface
vibegit commit --yolo --push     # ...and push afterwards
vibegit commit --yolo --unstage  # unstage already staged changes without asking
vibegit commit -i "group all test files together"
vibegit commit --debug           # write prompts and responses to $TMPDIR/vibegit-debug
```

In `--yolo` mode commits run with your terminal attached, so hooks, `gpg` prompts and editors behave as with plain `git commit`.

## Configuration

```bash
vibegit config          # setup wizard
vibegit config show     # print the configuration (keys masked)
vibegit config get model.name
vibegit config set model.name openai:gpt-5.6-terra
vibegit config set model.temperature none
vibegit config path
vibegit config open     # open the file in $EDITOR
```

The file lives at `~/Library/Application Support/vibegit/config.toml` on macOS, `~/.config/vibegit/config.toml` on Linux and `%LOCALAPPDATA%\vibegit\vibegit\config.toml` on Windows. `VIBEGIT_CONFIG_PATH` overrides the location. Existing configuration files from VibeGit 0.2 are read unchanged.

### Models

Model names use the `<provider>:<model>` format. Gemini 3.7 Flash (`google:gemini-3.8-flash`) is the default; the wizard also offers Gemini 3.5 Flash-Lite, Gemini 3.1 Pro, GPT-5.6 Terra/Sol/Luna and Grok Code Fast.

| Provider | Example | API key |
| --- | --- | --- |
| `google` | `google:gemini-3.8-flash` | `GOOGLE_API_KEY` (or `GEMINI_API_KEY`) |
| `openai` | `openai:gpt-5.6-terra` | `OPENAI_API_KEY` |
| `grok` | `grok:grok-code-fast-1` | `GROK_API_KEY` (or `XAI_API_KEY`) |
| `anthropic` | `anthropic:claude-...` | `ANTHROPIC_API_KEY` |
| `ollama` | `ollama:llama3` | none |
| `groq`, `deepseek`, `cohere`, `openrouter`, `together`, `fireworks` | `groq:...` | `GROQ_API_KEY`, ... |

API keys are read from the `api_keys` table in the configuration file first and from the environment second:

```bash
vibegit config set api_keys.OPENAI_API_KEY sk-...
```

Any OpenAI-compatible server works through `base_url` (the wizard's "Custom OpenAI-compatible endpoint" option):

```bash
vibegit config set model.name my-model
vibegit config set model.base_url https://api.example.com/v1
vibegit config set model.api_key <key>      # optional for local servers
```

The legacy `google-gla:`, `google_genai:` and `xai:` prefixes are still accepted.

### Options

| Path | Default | Meaning |
| --- | --- | --- |
| `model.name` | `google:gemini-3.8-flash` | Model in `<provider>:<model>` form |
| `model.temperature` | unset | Sampling temperature; `none` uses the provider default |
| `model.base_url`, `model.api_key` | unset | Custom OpenAI-compatible endpoint |
| `allow_excluding_changes` | `true` | Let the model set unfinished changes aside |
| `watermark` | `true` | Append "Commit generated by VibeGit" to messages |
| `context_formatting.include_active_branch` | `true` | Send the branch name |
| `context_formatting.include_latest_commits` | `5` | Recent messages sent as style reference |
| `context_formatting.truncate_lines` | `240` | Cut long diff lines before sending them |
| `context_formatting.max_file_size_kb` | `512` | Larger files are sent as a placeholder only |
| `confirmations.commit_all`, `.push`, `.rerun`, `.quit`, `.unstage` | `true` | Ask before committing all, pushing, regenerating, quitting with pending proposals, or unstaging. Pressing `Y` in the dialog sets the flag to `false`; set it back to `true` to ask again |

## Project rules (`.vibegitrules`)

A `.vibegitrules` file in the repository root customises proposals: message style, granularity, what to exclude. `vibegit init` creates a starter file (`--force` replaces an existing one). The `-i` flag and the `i` key provide one-off instructions that take priority over the file.

## What VibeGit does to your repository

* The working tree is never written. Only `git apply --cached`, `git reset -- <paths>`, `git commit` and (on request) `git push` run; every commit contains exactly the bytes in your files, regardless of `apply.whitespace` or other settings.
* An index that already contains staged changes is unstaged first, after you confirm. Intent-to-add entries count as staged.
* A merge, rebase, cherry-pick, revert or bisect in progress, and unmerged paths, stop VibeGit before anything happens.
* Submodule pointer updates are committed as one unit. Changes inside a submodule are reported and left alone; commit them in the submodule first.
* Nested repositories (an untracked directory with its own `.git`) are skipped with a warning; use `git submodule add` to track them.
* Untracked files above 50 MB are skipped with a warning; files above `max_file_size_kb` are committed as a whole but their content is not sent to the model.
* Detached and unborn HEAD work; the first commit of a new repository can be created by VibeGit.

## Troubleshooting

`vibegit commit --debug` writes the prompt, the model's answer and any error to `vibegit-debug/` in your temporary directory and prints full error chains. A failed commit (for example a rejected pre-commit hook) leaves the index clean; the proposal shows the hook output and can be retried with `c`.

## Development

```bash
cargo test                          # unit tests and git edge-case scenarios
cargo clippy --all-targets -- -D warnings
cargo fmt --check
uvx maturin build --release         # wheel in target/wheels/
uvx --from target/wheels/*.whl vibegit --version
```

Releases are published by the `Publish to PyPI` workflow, which builds wheels for Linux (x86_64, aarch64), macOS (x86_64, arm64) and Windows on a GitHub release. The design rationale for the git layer, including every edge case covered by the test-suite, is in `docs/specs/git-layer-refactor.md`.

## License

VibeGit is available under the MIT License. See [LICENSE](LICENSE).
