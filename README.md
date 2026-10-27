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

During a long coding session, unrelated changes pile up in the same working tree. Turning them into small, coherent commits means inspecting hunks, staging them one by one and writing matching messages. VibeGit does that for you:

```bash
vibegit commit
```

It reads the diff, the current branch and the recent history, asks a model for semantically related groups of hunks with commit messages, and opens an interactive review where you edit, reorder, skip and apply them.

VibeGit is a single native binary that drives the `git` command line. It never modifies your working tree: it only stages exactly the hunks it was told to and creates commits, so every commit contains precisely the bytes in your files.

## Features

* **Semantic grouping** of related hunks into commits, including changes that span several files.
* **Generated messages** that follow the style of the repository, guided by a `.vibegitrules` file derived from your own history.
* **Interactive review** with a diff viewer, in-place or `$EDITOR` message editing, reordering, skipping, commit-and-push in one step, and a button bar with mouse support; press `?` for all keys.
* **Change exclusions:** changes that look unfinished or sensitive are set aside and shown separately, never committed silently.
* **Many providers:** Google Gemini, OpenAI, xAI Grok, Anthropic, Ollama, Groq, DeepSeek and others, plus any OpenAI-compatible endpoint.
* **Robust git handling:** binaries, mode changes, symlinks, submodules, CRLF, unicode paths, detached and unborn HEAD, and unusual `diff.*` configuration all work; merges in progress and conflicts are refused before anything is written.

## Installation

```bash
uv tool install vibegit     # or: pipx install vibegit, pip install vibegit
```

Prebuilt wheels contain the binary, so no Rust toolchain or Python dependencies are needed. `uvx vibegit` runs it without installing. Git 2.25 or newer must be on your `PATH`. To build from source, run `cargo install --path .`.

## Quick start

```bash
vibegit config   # pick a model and store its API key
vibegit init     # derive .vibegitrules from the project's history and review it
vibegit commit   # review and apply proposed commits
```

Both setup steps also happen on demand: the wizard runs when no configuration exists, and `vibegit commit` offers to derive `.vibegitrules` when a repository has none.

## Commands

**`vibegit commit`** opens the review interface. Useful flags:

```bash
vibegit commit -i "group all test files together"   # one-off instruction
vibegit commit --yolo [--push] [--unstage]           # apply every proposal without the interface
vibegit commit --debug                               # dump prompts and responses to $TMPDIR/vibegit-debug
```

**`vibegit init`** analyses the last `init.commits` commits (messages and abbreviated diffs), derives the project's conventions into `.vibegitrules`, and shows the result for review before writing it. `--yes` skips the review, `--print` prints instead of writing, `--template` writes the bundled template, `-n N` changes the number of commits, `--force` replaces an existing file.

**`vibegit config`** runs the setup wizard; `show`, `get <path>`, `set <path> <value>`, `path` and `open` manage the file directly, for example:

```bash
vibegit config set model.name openai:gpt-5.6-terra
vibegit config set api_keys.OPENAI_API_KEY sk-...
```

## Configuration

The file lives in the platform configuration directory (`~/Library/Application Support/vibegit/config.toml` on macOS, `~/.config/vibegit/config.toml` on Linux); `VIBEGIT_CONFIG_PATH` overrides it. Files from VibeGit 0.2 are read unchanged.

Models use the `<provider>:<model>` format, with `google:gemini-3.8-flash` as the default. Keys are read from the `api_keys` table or the usual environment variables (`GOOGLE_API_KEY`, `OPENAI_API_KEY`, `GROK_API_KEY`, `ANTHROPIC_API_KEY`, ...). Any OpenAI-compatible server works through `model.base_url` and `model.api_key`.

Other settings: `allow_excluding_changes`, `watermark`, `context_formatting.*` (branch, recent commits, line truncation, size cap), `init.*` (commits analysed and diff detail for `vibegit init`), and `confirmations.*`. Each confirmation dialog has a "don't ask again" choice that is stored there. Run `vibegit config show` to see every setting with its current value.

## Project rules

`.vibegitrules` in the repository root tells VibeGit how commits should look: message style, granularity, grouping, exclusions. `vibegit init` writes it from your history; edit it freely. A `-i` instruction takes priority over the file for one run.

## Safety

* Only `git apply --cached`, `git reset -- <paths>`, `git commit` and, on request, `git push` run. The working tree is never written.
* Staged changes are unstaged first, after confirmation. Merges, rebases, cherry-picks and conflicts stop VibeGit before anything happens.
* Changes inside submodules and nested repositories are reported and left alone; files above 50 MB are skipped.
* A failed commit (for example a rejected hook) leaves the index clean and can be retried.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
uvx maturin build --release        # wheel in target/wheels/
```

Releases publish wheels for Linux, macOS and Windows through the `Publish to PyPI` workflow. The design notes for the git layer are in `docs/specs/git-layer-refactor.md`.

## License

MIT, see [LICENSE](LICENSE).
