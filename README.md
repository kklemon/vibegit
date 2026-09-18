<h1 align="center">VibeGit</h1>

<p align="center">
  <i>Turn a busy Git working tree into focused, reviewable commits.</i>
</p>

<p align="center">
  <a href="https://pypi.org/project/vibegit/" target="_blank">
    <img alt="PyPI - Version" src="https://img.shields.io/pypi/v/vibegit">
  </a>
  <a href="https://pypi.org/pypi/vibegit/" target="_blank">
    <img src="https://img.shields.io/badge/python-3.11%2B-blue" alt="py_versions">
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

During a long coding session, unrelated changes often accumulate in the same working tree. Turning them into small, coherent commits means repeatedly inspecting hunks, staging them, and writing matching commit messages.

Run VibeGit from the repository:

```bash
vibegit commit
```

VibeGit analyzes the diff, active branch, and recent commit history. It proposes semantically related groups of changes with generated commit messages, then lets you review or apply them.

> [!NOTE]
> VibeGit requires a repository with at least one commit. In a new repository, you can create one with `git commit --allow-empty -m "initial commit"`.

## Features

* **Semantic grouping:** Groups related hunks by their purpose, including changes that span multiple files.
* **Generated commit messages:** Suggests a concise message and explanation for each group.
* **Review controls:** Inspect proposals, edit messages, skip groups, or apply the remaining proposals automatically.
* **Configuration wizard:** Configures the model and API keys on first use.
* **Change exclusions:** Can leave out changes that appear unfinished, erroneous, or sensitive.

## Installation and setup

### Requirements

* [uv](https://docs.astral.sh/uv/getting-started/installation/) (recommended)
* Git
* Python 3.11 or newer when using another installation method

### Installation

Install VibeGit as an isolated command-line tool with uv:

```bash
uv tool install vibegit
```

uv makes the `vibegit` executable available on your `PATH` without mixing VibeGit's dependencies into your projects. If uv reports that its executable directory is not on `PATH`, run `uv tool update-shell` and restart your shell.

Upgrade VibeGit later with:

```bash
uv tool upgrade vibegit
```

To try VibeGit without installing it persistently, use `uv tool run` (or its `uvx` alias):

```bash
uv tool run vibegit --help
# Equivalent:
uvx vibegit --help
```

Alternative installation methods are also supported:

```bash
pipx install vibegit
# Or in a dedicated virtual environment:
pip install vibegit
```

### Quick start

Run these commands from your Git repository after installing VibeGit:

```bash
vibegit init    # Add a starter .vibegitrules file
vibegit config  # Choose a model and configure its API key
vibegit commit  # Analyze changes and create semantic commits
```

### First-run configuration

When you run VibeGit for the first time, it will launch an interactive configuration wizard to help you set up the most important settings:

- Choose an LLM model (Gemini, GPT, or custom)
- Configure the necessary API keys

```bash
# The wizard runs automatically on first use and whenever you run:
vibegit config

# Legacy alias (equivalent to the command above):
vibegit config wizard
```
Google Gemini is the default provider and requires a Google AI Studio API key. You can create one in [Google AI Studio](https://aistudio.google.com/app/apikey).

Selecting **Custom model (OpenAI API compatible)** lets you point VibeGit at any endpoint that implements the OpenAI Chat Completions API. The wizard will collect the base URL, model name, and API key and store them so that future runs interact with your custom endpoint automatically.

Re-running the wizard with this option will pre-fill the previously saved base URL and model name, and you can choose whether to reuse or replace the stored API key.

## Configuration reference

Use `vibegit config show` to print the current configuration.

To change one value, use `vibegit config set <path> <value>` with a dot-separated path such as `model.name`.

Use `vibegit config open` to edit the complete configuration file in your system's default editor.

Run `vibegit config` at any time to start the wizard again.

Below is a description of the most relevant configuration options.

### Models

Gemini 3.7 Flash is used by default. It is Google's latest stable Flash model, is designed for complex coding and reliable multi-step work, and supports structured outputs. You can use any other model that supports structured outputs given a JSON schema.

The configuration wizard recommends these current general-purpose models:

* Gemini 3.7 Flash (`google:gemini-3.8-flash`) — recommended default
* Gemini 3.5 Flash-Lite (`google:gemini-3.5-flash-lite`) — fastest, cost-efficient Gemini option
* Gemini 3.1 Pro (preview) (`google:gemini-3.1-pro-preview`) — advanced problem solving
* GPT-5.6 Terra (`openai:gpt-5.6-terra`) — balanced intelligence and cost
* GPT-5.6 Sol (`openai:gpt-5.6-sol`) — highest-quality complex reasoning and coding
* GPT-5.6 Luna (`openai:gpt-5.6-luna`) — cost-sensitive, high-volume work
* Grok Code Fast (`grok:grok-code-fast-1`)

VibeGit installs Pydantic AI Slim with the Google and OpenAI extras. The OpenAI extra also supports Grok and custom OpenAI-compatible endpoints. Other Pydantic AI providers require you to install their corresponding optional dependency separately. Model names should be provided in the `provider:model` format (for example, `openai:gpt-5.6-terra` or `google:gemini-3.7-flash`). Legacy `google-gla:` and `google_genai:` model names are migrated automatically.

To configure a model, use the following command:

```bash
vibegit config set model.name <model-name>
```

For OpenAI-compatible endpoints you can also set values manually:

```bash
vibegit config set model.model_provider openai
vibegit config set model.base_url https://api.example.com/v1
vibegit config set model.api_key <your-api-key>
```

Provider-specific API keys can also be stored under the `api_keys` configuration field. For example, configure Grok with:

```bash
vibegit config set api_keys.GROK_API_KEY <your-api-key>
```

> [!NOTE]
> Model selection is currently global rather than repository-specific.

### Excluding changes

By default, VibeGit may exclude changes that appear unfinished, erroneous, or sensitive instead of forcing them into a commit proposal.

Control this behavior with:

```bash
vibegit config set allow_excluding_changes <true/false>
```

Use a `.vibegitrules` file to provide project-specific guidance for exclusions and commit grouping.

## Project rules (`.vibegitrules`)

Add a `.vibegitrules` file to the repository root to customize commit proposals. Typical uses include:

* Commit message style
* Commit scope and granularity
* Excluding certain files or changes, either on semantic grounds or based on filetype

Create a starter file based on VibeGit's own rules by running this in your project directory:

```bash
vibegit init
```

The command preserves an existing `.vibegitrules` file. Use `vibegit init --force` to replace it with the bundled template. See [VibeGit's `.vibegitrules` file](https://github.com/kklemon/vibegit/blob/master/.vibegitrules) for the current template.

### One-off instructions

Use the `--instruction` flag with `vibegit commit` to provide one-off custom instructions without modifying `.vibegitrules`:

```bash
vibegit commit -i "group all test files together"
vibegit commit -i "do not include changes related to the cli"
```

This is useful for temporary requirements or trying a different commit style.

## Roadmap

VibeGit currently changes Git history only through the `commit` workflow; `init` and `config` are setup utilities. Possible future workflows include:

* `vibegit merge` for conflict resolution
* `vibegit rebase` for interactive rebase suggestions
* `vibegit checkout` for relevant branch suggestions

## Contributing

Bug reports, feature suggestions, and pull requests are welcome.

Pull requests and pushes are checked by CI. Run the same quality checks locally with:

```bash
uv sync --locked
uv run ruff check .
uv run ruff format --check .
uv run ty check
uv run pytest -q
```

## License

VibeGit is available under the MIT License. See [LICENSE](LICENSE).
