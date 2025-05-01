<h1 align="center">✨ VibeGit ✨</h1>

<p align="center">
Spend more time vibe coding and less time cleaning your messy git repository.
</p>

---

<p float="right" align="center">
    <img src="resources/before-vibegit.png" width="35%">
    <img src="resources/after-vibegit.png" width="35%">
</p>

<p align="center">
^ You before discovering VibeGit
&emsp;&emsp;&emsp;&emsp;&emsp;&emsp;&emsp;&emsp;^ You after discovering VibeGit
</p>

---

## Never ever do manual Git housekeeping again

Let's be honest. You know the problem. You spent hours or days working on a feature and forgot to group and commit changes once in a while. Suddenly you are facing 30 open file changes, scattered across a dozen or so different subtasks.

Now comes the fun part: **Crafting perfect, atomic commits.**

You could:

1.  Spend 20 minutes meticulously using `git add -p`, squinting at diffs like a code archaeologist.
2.  Write a vague commit message like `"fix stuff"` and promise yourself you'll `rebase -i` later (spoiler: you won't).
3.  Just `git commit -a -m "WIP"` and call it a day, leaving a dumpster fire for future you (or your poor colleagues).

**There *has* to be a better way.** (Spoiler: There is. Keep reading.)

## Enter VibeGit: Your AI-Powered Git Housekeeper 🤖🧹

> [!WARNING]
> Brace yourself. What you're about to see now might feel like magic.

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
*   🤖 **Multiple Workflow Modes:**
    *   **YOLO Mode:** Feeling lucky? Automatically apply all of VibeGit's proposals. What could possibly go wrong?
    *   **Interactive Mode:** Review each proposed commit, edit the message in your default editor, and apply them one by one. For the cautious (or skeptical).
    *   **Summary Mode:** Get a quick overview of what VibeGit plans to do before diving in.
*   🚫 **Sanity Check (coming soon™):** Checks whether you didn't accidentally forgot your life pension in form of a Bitcoin key in your `.env` file. 

## Setup: Get Ready to Vibe

### Requirements

* A computer
* Python>=3.10

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

Before your first vibe git'ing session, you may have to configure VibeGit. This can be done with the `vibegit config set <path> <value>` CLI command. You will most likely have to provide your LLM API key. Google's Gemini models are used by default, so you have will to set `api_keys.google_api_key` to your API key:

```bash
vibegit config set api_keys.google_api_key <your-secret-api-key>
```

If you don't have a Gemini API key yet, get one [here](https://aistudio.google.com/app/apikey).

## The Future: More Vibes, More Git? 🚀

Right now, VibeGit focuses on `commit`. But the vision is grand! Imagine AI assistance for:

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
