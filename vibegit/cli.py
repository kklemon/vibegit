from copy import deepcopy
import os
import subprocess
import sys
from pathlib import Path

import click
import git
import inquirer
from rich import print as pprint
from rich.table import Table
from rich.console import Console

from vibegit.ai import CommitProposalAI
from vibegit.config import Config, CONFIG_PATH
from vibegit.git import (
    CommitProposalContext,
    get_git_status,
)
from vibegit.schemas import (
    CommitProposalListSchema,
    IncompleteCommitProposalListSchema,
)

# Temporary fix. See https://github.com/grpc/grpc/issues/37642
# Update: Doesn't seem to work.
os.environ["GRPC_VERBOSITY"] = "NONE"

console = Console()

def get_config() -> Config:
    try:
        config = Config()
        return config
    except Exception as e:
        console.print(f"[bold red]Error loading config: {e}[/bold red]")

        questions = [
            inquirer.Confirm(
                "reset",
                message="Reset config or exit?",
                default=False,
            ),
        ]
        answers = inquirer.prompt(questions)

        if answers and answers["reset"]:
            CONFIG_PATH.unlink()
            console.print("[green]Config reset successfully.[/green]")
            return get_config()
        else:
            console.print("[red]Exiting.[/red]")
            sys.exit(1)


config = get_config()


def has_staged_changes(repo: git.Repo) -> bool:
    """Check if there are any changes staged in the Git index."""
    try:
        # diff('HEAD') compares the index (staging area) with the last commit
        staged_diff = repo.index.diff("HEAD")
        return bool(staged_diff)
    except git.GitCommandError as e:
        console.print(f"[bold red]Error checking for staged changes: {e}[/bold red]")
        return False  # Assume no staged changes if check fails? Or maybe re-raise?


def reset_staged_changes(repo: git.Repo) -> bool:
    """Reset (unstage) all changes from the Git index."""
    try:
        # 'git reset HEAD --' unstages all changes
        repo.git.reset("HEAD", "--")
        if has_staged_changes(repo):
            console.print("[red]Failed to reset staged changes.[/red]")
            return False
        console.print("[green]Successfully reset staged changes.[/green]")
        return True
    except git.GitCommandError as e:
        console.print(f"[bold red]Error resetting staged changes: {e}[/bold red]")
        return False


def display_summary(
    proposals: CommitProposalListSchema,
):
    """Displays a summary of the commit proposals."""
    if not proposals or not proposals.commit_proposals:
        console.print("[yellow]No commit proposals to display.[/yellow]")
        return

    table = Table(title="Commit Proposals Summary")
    table.add_column("No.", style="dim", width=3)
    table.add_column("Proposed Message", style="cyan", no_wrap=False)
    table.add_column("Changes", style="magenta")
    table.add_column("Reasoning", style="yellow", no_wrap=False)

    for i, proposal in enumerate(proposals.commit_proposals):
        table.add_row(
            str(i + 1),
            proposal.commit_message,
            ", ".join(map(str, proposal.change_ids)),  # Change IDs are ints now
            proposal.reasoning,
        )

    console.print(table)

    if (
        isinstance(proposals, IncompleteCommitProposalListSchema)
        and proposals.exclude.change_ids
    ):
        console.print()

        table = Table(title="Excluded Changes")
        table.add_column("Changes", style="magenta")
        table.add_column("Reasoning", style="yellow", no_wrap=False)
        table.add_row(
            ", ".join(map(str, proposals.exclude.change_ids)),
            proposals.exclude.reasoning,
        )

        console.print(table)


def open_editor_for_commit(repo: git.Repo, proposed_message: str) -> bool:
    """
    Runs 'git commit -e -m <proposed_message>' to open the default editor
    allowing the user to finalize the commit message.
    Returns True if the command exits successfully (exit code 0), False otherwise.
    """
    try:
        # Use subprocess to run git commit with -e (edit) and -m (message)
        # Git will use the EDITOR or VISUAL environment variable, or fallback
        env = os.environ.copy()
        result = subprocess.run(
            ["git", "commit", "-e", "-m", proposed_message],
            cwd=repo.working_dir,
            env=env,
            check=False,  # Don't raise exception on non-zero exit
        )
        if result.returncode == 0:
            console.print("[green]Commit successful (via editor).[/green]")
            return True
        else:
            console.print(
                f"[yellow]Commit aborted or failed in editor (exit code: {result.returncode}).[/yellow]"
            )
            # Consider unstaging here if desired: repo.git.reset("HEAD", "--")
            return False
    except FileNotFoundError:
        console.print(
            "[bold red]Error: 'git' command not found. Is Git installed and in your PATH?[/bold red]"
        )
        return False
    except Exception as e:
        console.print(
            f"[bold red]An unexpected error occurred while trying to open the commit editor: {e}[/bold red]"
        )
        return False


def get_user_instructions(repo: git.Repo) -> str | None:
    path = Path(repo.working_dir) / ".vibegitrules"
    if path.exists():
        return path.read_text()
    return None


def prepare_repo(repo: git.Repo):
    if has_staged_changes(repo):
        console.print("[bold yellow]Warning:[/bold yellow] Found staged changes.")
        console.print(
            "VibeGit works best with unstaged changes only, as it needs to stage changes itself."
        )

        questions = [
            inquirer.Confirm(
                "reset",
                message="Do you want to unstage (reset) all currently staged changes?",
                default=False,
            ),
        ]
        answers = inquirer.prompt(questions)

        if answers and answers["reset"]:
            if not reset_staged_changes(repo):
                # Try one more time? The prompt requested exiting if first attempt fails.
                console.print(
                    "[bold red]Failed to reset staged changes. Exiting.[/bold red]"
                )
                sys.exit(1)
            else:
                console.print("Staged changes have been reset. Proceeding...")
        else:
            console.print("Cannot proceed with staged changes. Exiting.")
            sys.exit(0)
    else:
        console.print("[green]Repository has no staged changes. Good to go![/green]")


def run_commit_workflow(repo: git.Repo):
    """Handles the main logic for the 'commit' subcommand."""
    console.print("[bold blue]VibeGit Commit Workflow Starting...[/bold blue]")

    # 1. Check for staged changes
    prepare_repo(repo)

    # 2. Get Git Status and check for *any* changes
    try:
        status = get_git_status(repo)
        if not status.changed_files and not status.untracked_files:
            console.print(
                "[yellow]No unstaged changes or untracked files found to process. Exiting.[/yellow]"
            )
            sys.exit(0)
        console.print(
            f"Found {len(status.changed_files)} changed and {len(status.untracked_files)} untracked files."
        )
    except Exception as e:
        console.print(f"[bold red]Error getting Git status: {e}[/bold red]")
        sys.exit(1)

    # 3. Prepare AI Context
    formatter = config.context_formatting.get_context_formatter(
        user_instructions=get_user_instructions(repo)
    )
    ctx = CommitProposalContext(git_status=status)

    console.print("Formatting changes for AI analysis...")
    try:
        formatted_context = formatter.format_changes(ctx)
        # print(formatted_context) # Debugging: Uncomment to see what's sent to the LLM
    except Exception as e:
        console.print(f"[bold red]Error formatting changes for AI: {e}[/bold red]")
        sys.exit(1)

    if not ctx.change_id_to_ref:
        console.print(
            "[yellow]No detectable changes found in the changes. Cannot generate proposals. Exiting.[/yellow]"
        )
        sys.exit(0)

    console.print(f"Identified {len(ctx.change_id_to_ref)} change(s).")

    # 4. Get Commit Proposals from AI
    console.print("Generating commit proposals...")
    ai = CommitProposalAI(
        config.model.get_chat_model(),
        allow_excluding_changes=config.allow_excluding_changes,
    )
    result: CommitProposalListSchema | IncompleteCommitProposalListSchema | None = None

    try:
        result = ai.propose_commits(formatted_context)
    except Exception as e:
        console.print(
            f"[bold red]Error getting commit proposals from AI: {e}[/bold red]"
        )
        # Consider more specific error handling based on potential AI exceptions
        sys.exit(1)

    if not result or not result.commit_proposals:
        console.print(
            "[yellow]AI did not generate any commit proposals. Exiting.[/yellow]"
        )
        sys.exit(0)

    console.print(
        f"[green]Generated {len(result.commit_proposals)} commit proposal(s).[/green]"
    )

    # 5. Validate Proposals
    try:
        ctx.validate_commit_proposal(result)
        console.print("[green]AI proposals validated successfully.[/green]")
    except ValueError as e:
        print(formatted_context)
        console.print(f"[bold red]AI proposal validation failed: {e}[/bold red]")
        console.print("Cannot proceed with invalid proposals. Exiting.")
        sys.exit(1)

    # 6. Interactive Workflow Choice
    questions = [
        inquirer.List(
            "mode",
            message="How do you want to proceed?",
            choices=[
                ("Apply all proposed commits automatically (#yolo)", "yolo"),
                (
                    "Interactive: Review and commit each proposal one by one (opens editor)",
                    "interactive",
                ),
                ("Summary: Show a summary of all proposals first", "summary"),
                ("Rerun VibeGit", "rerun"),
                ("Quit: Exit without applying any proposals", "quit"),
            ],
            default="interactive",
        ),
    ]
    answers = inquirer.prompt(questions)
    mode = answers["mode"] if answers else "quit"

    if mode == "quit":
        console.print("[yellow]Exiting as requested.[/yellow]")
        sys.exit(0)

    if mode == "rerun":
        console.print("[yellow]Rerunning VibeGit...[/yellow]")
        run_commit_workflow(repo)
        sys.exit(0)

    if mode == "summary":
        display_summary(result)

        # After summary, ask again how to proceed (excluding summary itself)
        questions = [
            inquirer.List(
                "mode_after_summary",
                message="How do you want to proceed now?",
                choices=[
                    ("Apply all proposed commits automatically (#yolo)", "yolo"),
                    (
                        "Interactive: Review and commit one by one (opens editor)",
                        "interactive",
                    ),
                    (
                        "Show a detailed summary of the commit proposals",
                        "detailed_summary",
                    ),
                    ("Quit", "quit"),
                ],
                default="interactive",
            ),
        ]
        answers = inquirer.prompt(questions)
        mode = answers["mode_after_summary"] if answers else "quit"
        if mode == "quit":
            console.print("[yellow]Exiting as requested.[/yellow]")
            sys.exit(0)

    if mode == "yolo":
        commit_proposals = result.commit_proposals
        console.print(
            f"\n[bold magenta]Entering #yolo Mode: Applying all {len(commit_proposals)} proposals...[/bold magenta]"
        )
        original_count = len(commit_proposals)
        for i, proposal in enumerate(list(commit_proposals)):
            console.print(
                f"\nApplying proposal {i + 1} of {original_count}: '{proposal.commit_message}'"
            )
            console.print(f"  Changes: {proposal.change_ids}")
            try:
                console.print("[cyan]Staging changes...[/cyan]")
                ctx.stage_commit_proposal(proposal)
                console.print("[green]Changes staged successfully.[/green]")

                console.print("[cyan]Creating commit...[/cyan]")
                # In YOLO mode, commit directly without opening editor
                ctx.commit_commit_proposal(proposal)
                console.print("[green]Commit created successfully.[/green]")
                commit_proposals.pop(0)  # Remove applied proposal from original list

            except git.GitCommandError as e:
                console.print(
                    f"[bold red]Error during Git operation for proposal {i + 1}: {e}[/bold red]"
                )
                console.print(
                    "[bold yellow]Stopping Yolo mode due to error. Remaining proposals are not applied.[/bold yellow]"
                )
                # Attempt to unstage potentially problematic changes?
                console.print(
                    "[cyan]Attempting to unstage changes from failed step...[/cyan]"
                )
                reset_staged_changes(repo)
                break
            except Exception as e:
                console.print(
                    f"[bold red]An unexpected error occurred processing proposal {i + 1}: {e}[/bold red]"
                )
                console.print(
                    "[bold yellow]Stopping Yolo mode due to error. Remaining proposals are not applied.[/bold yellow]"
                )
                # Attempt to unstage potentially problematic changes?
                console.print(
                    "[cyan]Attempting to unstage changes from failed step...[/cyan]"
                )
                reset_staged_changes(repo)

                raise e

        result.commit_proposals = commit_proposals

    elif mode == "interactive":
        console.print("\n[bold magenta]Entering Interactive Mode...[/bold magenta]")
        result_copy = deepcopy(result)
        total_proposals = len(result_copy.commit_proposals)
        committed_count = 0

        while result_copy.commit_proposals:
            proposal = result_copy.commit_proposals[0]

            current_num = total_proposals - len(result_copy.commit_proposals) + 1

            console.print("\n" + "=" * 40)
            console.print(f"[bold]Proposal {current_num} of {total_proposals}:[/bold]")
            display_summary(result_copy)

            questions = [
                inquirer.List(
                    "action",
                    message="Choose an action for this proposal:",
                    choices=[
                        ("Commit (opens editor)", "commit"),
                        ("Skip this proposal for now", "skip"),
                        ("Apply All remaining proposals (Yolo)", "all"),
                        ("Show Summary of remaining proposals", "summary"),
                        ("Quit", "quit"),
                    ],
                    default="commit",
                ),
            ]
            answers = inquirer.prompt(questions)
            action = answers["action"] if answers else "quit"

            if action == "commit":
                try:
                    console.print("[cyan]Staging changes for commit...[/cyan]")
                    ctx.stage_commit_proposal(proposal)
                    console.print("[green]Changes staged.[/green]")

                    console.print("[cyan]Opening editor for commit message...[/cyan]")
                    commit_successful = open_editor_for_commit(
                        repo, proposal.commit_message
                    )

                    if commit_successful:
                        committed_count += 1
                        result_copy.commit_proposals.pop(0)  # Remove if committed
                        console.print(
                            f"[green]Proposal {current_num} committed.[/green]"
                        )
                    else:
                        console.print(
                            "[yellow]Commit was cancelled or failed. Staged changes remain.[/yellow]"
                        )
                        console.print(
                            "[yellow]You may want to manually commit or reset changes.[/yellow]"
                        )
                        # Ask user if they want to reset the staged changes from this failed attempt
                        q_reset = [
                            inquirer.Confirm(
                                "reset_failed",
                                message="Unstage the changes from this aborted commit?",
                                default=True,
                            )
                        ]
                        a_reset = inquirer.prompt(q_reset)
                        if a_reset and a_reset["reset_failed"]:
                            reset_staged_changes(repo)
                        # Decide whether to continue or quit on failure? Let's continue for now.
                        # Optionally: Move skipped proposal to the end? For now, just keeps it at the front for next loop.
                        # To skip properly, we'd pop and potentially store elsewhere. Let's add a 'skip' choice.

                except git.GitCommandError as e:
                    console.print(
                        f"[bold red]Error staging changes for proposal {current_num}: {e}[/bold red]"
                    )
                    console.print(
                        "[yellow]Skipping this proposal due to staging error.[/yellow]"
                    )
                    # Do not remove the proposal, let user decide next iteration or quit
                except Exception as e:
                    console.print(
                        f"[bold red]An unexpected error occurred processing proposal {current_num}: {e}[/bold red]"
                    )
                    console.print(
                        "[yellow]Skipping this proposal due to unexpected error.[/yellow]"
                    )

            elif action == "skip":
                console.print(
                    f"[yellow]Skipping proposal {current_num}. It will be shown again later if you continue.[/yellow]"
                )
                # Move proposal to the end of the list to avoid immediate repetition
                skipped_proposal = result_copy.commit_proposals.pop(0)
                result_copy.commit_proposals.append(skipped_proposal)

            elif action == "all":
                console.print(
                    f"\n[bold magenta]Switching to Yolo Mode for the remaining {len(result_copy.commit_proposals)} proposals...[/bold magenta]"
                )
                initial_remaining_count = len(result_copy.commit_proposals)
                yolo_successful = True
                for i, p in enumerate(list(result_copy.commit_proposals)):
                    console.print(
                        f"\nApplying remaining proposal {i + 1} of {initial_remaining_count}: '{p.commit_message}'"
                    )
                    console.print(f"  Changes: {p.change_ids}")
                    try:
                        console.print("[cyan]Staging changes...[/cyan]")
                        ctx.stage_commit_proposal(p)
                        console.print("[green]Changes staged successfully.[/green]")
                        console.print("[cyan]Creating commit...[/cyan]")
                        repo.index.commit(p.commit_message)  # Yolo -> No editor
                        console.print("[green]Commit created successfully.[/green]")
                        result_copy.commit_proposals.pop(0)  # Remove from original list
                        committed_count += 1
                    except git.GitCommandError as e:
                        console.print(
                            f"[bold red]Error during Git operation for proposal: {e}[/bold red]"
                        )
                        console.print(
                            "[bold yellow]Stopping Yolo mode due to error.[/bold yellow]"
                        )
                        console.print(
                            "[cyan]Attempting to unstage changes from failed step...[/cyan]"
                        )
                        reset_staged_changes(repo)
                        yolo_successful = False
                        break
                    except Exception as e:
                        console.print(
                            f"[bold red]An unexpected error occurred processing proposal: {e}[/bold red]"
                        )
                        console.print(
                            "[bold yellow]Stopping Yolo mode due to error.[/bold yellow]"
                        )
                        console.print(
                            "[cyan]Attempting to unstage changes from failed step...[/cyan]"
                        )
                        reset_staged_changes(repo)
                        yolo_successful = False
                        break
                if not yolo_successful:
                    console.print(
                        "[yellow]Finished applying remaining proposals (with errors).[/yellow]"
                    )
                else:
                    console.print(
                        "[green]Successfully applied all remaining proposals.[/green]"
                    )
                break  # Exit interactive loop after 'all' attempt

            elif action == "summary":
                display_summary(result_copy)
                # Loop continues to show the current proposal again

            elif action == "quit":
                console.print("[yellow]Quitting interactive mode.[/yellow]")
                break  # Exit the while loop

        # End of interactive loop
        if not result_copy.commit_proposals:
            console.print("\n[bold green]All proposals processed.[/bold green]")
        else:
            console.print(
                f"\n[yellow]Exited with {len(result_copy.commit_proposals)} proposals remaining.[/yellow]"
            )

    # --- Final Summary ---
    final_status = get_git_status(repo)
    if not final_status.changed_files and not final_status.untracked_files:
        # Check if *staged* changes exist from failed editor commit etc.
        if not has_staged_changes(repo):
            console.print(
                "\n[bold green]VibeGit finished. Working directory is clean. 😎[/bold green]"
            )
        else:
            console.print(
                "\n[bold yellow]VibeGit finished. There are still staged changes remaining.[/bold yellow]"
            )
    else:
        console.print(
            "\n[bold yellow]VibeGit finished. There are still unstaged changes or untracked files.[/bold yellow]"
        )


def run_commit(debug: bool = False):
    # For now, only the 'commit' subcommand is implemented directly.
    # Later, this could use argparse or Typer/Click to handle subcommands.
    # Example: if args.subcommand == 'commit': await run_commit_workflow()

    # Find Git repository
    try:
        repo = git.Repo(os.getcwd(), search_parent_directories=True)
        console.print(f"Found Git repository at: {repo.working_dir}")
    except git.InvalidGitRepositoryError:
        console.print("[bold red]Error: Invalid Git repository detected.[/bold red]")
        sys.exit(1)
    except Exception as e:
        console.print(
            f"[bold red]Error initializing Git repository object: {e}[/bold red]"
        )
        sys.exit(1)

    try:
        run_commit_workflow(repo)
    except KeyboardInterrupt:
        console.print("\n[yellow]Operation cancelled by user.[/yellow]")
        sys.exit(1)
    except Exception as e:
        # Catch-all for unexpected errors during async execution
        console.print(f"[bold red]An unexpected error occurred: {e}[/bold red]")

        if debug:
            # Optionally print traceback here for debugging
            import traceback

            traceback.print_exc()
            sys.exit(1)


@click.group(invoke_without_command=True)
@click.pass_context
def cli(ctx):
    if not ctx.invoked_subcommand:
        console.print(
            "[bold yellow]WARNING: If no command is provided, VibeGit will run the commit workflow. This is due to change. We recommend running VibeGit with the 'commit' command explicitly.[/bold yellow]"
        )
        run_commit()


@cli.command()
@click.option("--debug", "-d", is_flag=True, help="Enable debug mode")
def commit(debug: bool):
    run_commit(debug)


@cli.group(name="config", invoke_without_command=True)
@click.pass_context
def config_cli(ctx):
    if not ctx.invoked_subcommand:
        pprint(config)


@config_cli.command()
def open():
    import subprocess

    subprocess.run(["open", CONFIG_PATH])


@config_cli.command()
@click.argument("path", type=str)
def get(path: str):
    pprint(config.get_by_path(path))


@config_cli.command()
@click.argument("path", type=str)
@click.argument("value", type=str)
def set(path: str, value: str):
    config.set_by_path(path, value)
    config.save_config()


if __name__ == "__main__":
    cli()
