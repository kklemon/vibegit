<h1 align="center">✨ VibeGit ✨</h1>

<p align="center">
  <i>Spend more time (vibe) coding and less time cleaning your messy git repository.</i>
</p>

<p align="center">
  <a href="https://pypi.org/project/vibegit/" target="_blank">
    <img alt="PyPI - Version" src="https://img.shields.io/pypi/v/vibegit">
  </a>
  <a href="https://pypi.org/pypi/vibegit/" target="_blank">
    <img src="https://img.shields.io/badge/python-3.11%2B-blue" alt="py_versions">
  </a>
</p>

---

<p float="right" align="center">
    <img src="resources/before-vibegit.png" width="45%">
    <img src="resources/after-vibegit.png" width="45%">
</p>

<p align="center">
^ You before discovering VibeGit
&emsp;&emsp;&emsp;&emsp;&emsp;&emsp;&emsp;&emsp;^ You after discovering VibeGit
</p>

---

## Never ever do manual Git housekeeping again

Let's be honest. You know the problem. You spent hours or days working on a feature and forgot to group and commit changes once in a while. Suddenly you are facing 30 open file changes, related to a dozen or so different subtasks.

Now comes the fun part: **Crafting perfect, atomic commits.**

You could:

1.  Spend 20 minutes meticulously using `git add -p`, squinting at diffs like a code archaeologist.
2.  Write a vague commit message like `"fix stuff"` and promise yourself you'll `rebase -i` later (spoiler: you won't).
3.  Just `git commit -a -m "WIP"` and call it a day, leaving a dumpster fire for future you (or your poor colleagues).

**There *has* to be a better way.**

## Enter VibeGit: Your AI-Powered Git Housekeeper 🤖🧹

> [!WARNING]
> Brace yourself. What you're about to see might blow your mind.

In your messy Git repository, just hit

```bash
vibegit commit
```

✨ **And it *automagically* groups related changes (hunks) together based on their *semantic meaning*!** ✨

No more manual patch-adding hell. No more "what did I even change here?" moments.

VibeGit analyzes your diff, considers your branch name, peeks at your recent commit history (for stylistic consistency, not blackmail... probably), and then proposes logical, beautifully grouped commits with **AI-generated commit messages**.

> [!NOTE]
> VibeGit currently only works if at least one commit exists. If you want to use it in a freshly initialized repository, you may create an empty commit with `git commit --allow-empty -m "initial commit"`.

## Features That Will Make You Question Reality (or at Least Your Old Workflow)

*   🧠 **Semantic Hunk Grouping:** VibeGit doesn't just look at file names; it looks at *what the code does* to bundle related changes. It's like magic, but with more AI slop.
*   ✍️ **AI-Generated Commit Messages:** Get sensible, well-formatted commit messages suggested for each group. Tweak them or use them as-is. Your commit log will suddenly look respectable.
*   🔧 **Interactive Configuration Wizard:** A friendly setup process for first-time users that helps configure your preferred AI model and API keys.
*   🤖 **Multiple Workflow Modes:**
    *   **YOLO Mode:** Feeling lucky? Automatically apply all of VibeGit's proposals. What could possibly go wrong?
    *   **Interactive Mode:** Review each proposed commit, edit the message in your default editor, and apply them one by one. For the cautious (or skeptical).
    *   **Summary Mode:** Get a quick overview of what VibeGit plans to do before diving in.
*   🚫 **Exclude Changes:** VibeGit will automatically exclude changes that shouldn't be committed such as API keys or unfinished work.

## Setup: Get Ready to Vibe

### Requirements

* A computer
* Python>=3.11

### Installation

Via pip:

```
pip install vibegit
```

Via pipx:

```
pipx install vibegit
```

**Run as tool without explicit installation with uv:**

```
uvx vibegit
```

### Configuration

When you run VibeGit for the first time, it will launch an interactive configuration wizard to help you set up the most important settings:

- Choose an LLM model (Gemini, GPT, or custom)
- Configure the necessary API keys

```bash
# The wizard runs automatically on first use, but you can manually run it anytime with:
vibegit config wizard
```
Google's Gemini models are used by default for which you will need a Google AI Studio API key. If you don't have a Gemini API key yet, get one [here](https://aistudio.google.com/app/apikey).

## Manual Configuration

Use `vibegit config` to print the current configuration to the console.

To set single configuration values, use `vibegit config set <path> <value>` and provide the configuration path in dot notation, e.g. `model.name`.

For a more convenient editing of the whole configuration file, use `vibegit config open` which will open the config file in your system's default editor.

Need to start over? Run the configuration wizard at any time with `vibegit config wizard` to reconfigure your settings.

Below is a description of the most relevant configuration options.

### Models

Gemini 2.5 Flash is used by default, as it provides arguably the best trade-off between performance, price and latency. However, you can use any model that supports structured outputs given a JSON schema.

VibeGit has been tested with:

* Gemini 2.5 Flash (`google_genai:gemini-2.5-flash`)
* Gemini 2.5 Pro (`google_genai:gemini-2.5-pro`)
* Grok Code Fast (`xai:grok-code-fast-1`)
* GPT 4o (`openai:gpt-4o`)
* GPT 4.1 (`openai:gpt-4.1`)
* o4-mini (`openai:o4-mini`)
* o3-mini (`openai:o3-mini`)

You can use any other model that meets the aforementioned requirements and is supported by LangChain. The model name needs to be provided in the [`init_chat_model` format](https://python.langchain.com/api_reference/langchain/chat_models/langchain.chat_models.base.init_chat_model.html).

To configure a model, use the following command:

```bash
vibegit config set model.name <model-name>
```

You may have to provide a provider-specific API key which can be done by setting the API key under the `api_keys` config field. For instance, to supply an API for xAI models, run the following command:

```bash
vibegit config set api_keys.XAI_API_KEY <your-api-key>
```

> [!NOTE]
> Models can't be configured on repository level at the moment.

### Incomplete Commit Proposals

VibeGit can be configured to generate commit proposals that include all open changes and exclude changes which may look unfinished or contain obvious errors (enabled by default).

To control this option, use

```bash
vibegit config set allow_excluding_changes <true/false>
```

The behavior of the excluding behavior can be customized with a `.vibegitrules` file (see next section).

## .vibegitrules

You may provide a `.vibegitrules` file in the root of your repository with custom instructions for the generation of commit proposals. Typical use cases are:

* Commit message style
* Commit scope and granularity
* Excluding certain files or changes, either on semantic grounds or based on filetype

See [VibeGit's `.vibegitrules` file](https://github.com/kklemon/vibegit/blob/master/.vibegitrules) for an example.

### Custom Instructions on the Fly

Use the `--instruction` flag with `vibegit commit` to provide one-off custom instructions without modifying `.vibegitrules`:

```bash
vibegit commit -i "group all test files together"
vibegit commit -i "do not include changes related to the cli"
```

This is handy for temporary requirements or experimenting with different commit styles.

## The Future: More Vibes, More Git? 🚀

At the moment VibeGit only supports the `commit` command. But the vision is grand! Imagine AI assistance for:

*   `vibegit merge` (Resolving conflicts? Maybe too ambitious...)
*   `vibegit rebase` (Interactive rebasing suggestions?)
*   `vibegit checkout` (Suggesting relevant branches?)

We're aiming to turn this quirky tool into a full-fledged AI Git companion.

## Contributing (Please Help Us Vibe Better!)

Found a bug? Have a killer feature idea? Did the AI `rm -rf`ed your repository once again?

Open an issue or submit a pull request! We appreciate constructive feedback and contributions. Let's make Git less of a chore, together.

## License

Currently under MIT License. Feel free to blatantly steal as much code as you want.

---

<p align="center">
  <b>Happy Vibing! ✨</b>
</p>
