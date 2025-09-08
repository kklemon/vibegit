import os
import subprocess
import sys
import tempfile
from copy import deepcopy
from pathlib import Path

import click
import git
from git.remote import PushInfo
import inquirer
from rich import print as pprint
from rich.console import Console
from rich.table import Table
from unidiff import PatchedFile

from vibegit.ai import CommitProposalAI
from vibegit.config import CONFIG_PATH, Config
from vibegit.git import (
    CommitProposalContext,
    FileDiff,
    GitStatusSummary,
    get_git_status,
)
from vibegit.schemas import (
    CommitProposalSchema,
    CommitProposalsResultSchema,
    IncompleteCommitProposalsResultSchema,
)
from vibegit.utils import compare_versions, get_version
from vibegit.wizard import run_wizard_if_needed

# Temporary fix. See https://github.com/grpc/grpc/issues/37642
# Update: Doesn't seem to work.
os.environ["GRPC_VERBOSITY"] = "NONE"

console = Console()


def check_for_update():
    try:
        import requests

        version = get_version()

        try:
            response = requests.get("https://pypi.org/pypi/vibegit/json", timeout=1)
            latest_version = response.json()["info"]["version"]

            if compare_versions(latest_version, version):
                console.print(
                    f"[bold yellow]You are using vibegit version {version}, however version {latest_version} is available. "
                    'You should consider upgrading via the "pip install --upgrade vibegit" command. '
                    "Otherwise you might miss out on important bug fixes and new features![/bold yellow]"
                )
        except (
            requests.exceptions.RequestException,
            requests.exceptions.ConnectionError,
            requests.exceptions.HTTPError,
            requests.exceptions.SSLError,
            requests.exceptions.Timeout,
        ):
            # when pypi servers or le internet is down
            pass
    except ModuleNotFoundError:
        pass


check_for_update()


def get_config() -> Config:
    # Check if this is first run and run wizard if needed
    run_wizard_if_needed()

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
            console.print("Running configuration wizard...")
            run_wizard_if_needed()  # Force wizard to run after reset
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


def get_project_instructions(repo: git.Repo) -> str | None:
    path = Path(repo.working_dir) / ".vibegitrules"
    if path.exists():
        return path.read_text()
    return None


class InteractiveCLI:
    def __init__(
        self, config: Config, repo: git.Repo, custom_instruction: str | None = None
    ):
        self.config = config
        self.repo = repo
        self.result: CommitProposalsResultSchema | None = None
        self.ctx: CommitProposalContext | None = None
        self.custom_instruction = custom_instruction

    def prepare_repo(self):
        if has_staged_changes(self.repo):
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
                if not reset_staged_changes(self.repo):
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
            console.print(
                "[green]Repository has no staged changes. Good to go![/green]"
            )

    def get_git_status(self):
        # 2. Get Git Status and check for *any* changes
        try:
            status = get_git_status(self.repo)
            if not status.changed_files and not status.untracked_files:
                console.print(
                    "[yellow]No unstaged changes or untracked files found to process. Exiting.[/yellow]"
                )
                sys.exit(0)
            console.print(
                f"Found {len(status.changed_files)} changed and {len(status.untracked_files)} untracked files."
            )
            return status
        except Exception as e:
            console.print(f"[bold red]Error getting Git status: {e}[/bold red]")
            sys.exit(1)

    def generate_commit_proposals(
        self, status: GitStatusSummary
    ) -> tuple[CommitProposalContext, CommitProposalsResultSchema | None]:
        formatter = self.config.context_formatting.get_context_formatter(
            project_instructions=get_project_instructions(self.repo),
            custom_instructions=self.custom_instruction,
        )
        ctx = CommitProposalContext(
            git_status=status, watermark_commits=self.config.watermark
        )

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

        console.print("Generating commit proposals...")
        ai = CommitProposalAI(
            config.model.get_chat_model(),
            allow_excluding_changes=config.allow_excluding_changes,
        )

        try:
            result = ai.propose_commits(formatted_context)
        except Exception as e:
            console.print(
                f"[bold red]Error getting commit proposals from AI: {e}[/bold red]"
            )
            sys.exit(1)

        if not result:
            console.print(
                "[red]Error: Model did not return any results. Exiting.[/red]"
            )
            sys.exit(1)

        if not result.commit_proposals:
            console.print(
                "[yellow]AI did not generate any commit proposals. Exiting.[/yellow]"
            )
            sys.exit(0)

        console.print(
            f"[green]Generated {len(result.commit_proposals)} commit proposal(s).[/green]"
        )

        try:
            ctx.validate_commit_proposal(result)
            console.print("[green]AI proposals validated successfully.[/green]")
        except ValueError as e:
            print(formatted_context)
            console.print(f"[bold red]AI proposal validation failed: {e}[/bold red]")
            console.print("Cannot proceed with invalid proposals. Exiting.")
            sys.exit(1)

        return ctx, result

    def _format_change_type(self, file: PatchedFile) -> str:
        if file.is_added_file:
            return "[green]A[/green]"
        elif file.is_removed_file:
            return "[red]D[/red]"
        elif file.is_modified_file:
            return "[yellow]M[/yellow]"
        else:
            return "[blue]U[/blue]"

    def _format_file(self, file_diff: FileDiff) -> str:
        patched_file = file_diff.patched_file
        if patched_file.is_binary_file:
            summary = "binary"
        else:
            lines_added = lines_removed = 0

            for hunk in file_diff.patched_file:
                lines_added += hunk.added
                lines_removed += hunk.removed

            summary = f"[green]+{lines_added}[/green], [red]-{lines_removed}[/red]"

        return (
            f"{self._format_change_type(patched_file)} {patched_file.path} ({summary})"
        )

    def _format_commit_proposal_changes(self, change_ids: list[int]) -> str:
        file_diffs = self.ctx.get_file_diffs_from_change_ids(change_ids)

        files = [self._format_file(file) for file in file_diffs]

        return "\n".join(files)

    def display_commit_proposals_summary(
        self,
    ):
        """Displays a summary of the commit proposals."""
        if not self.result or not self.result.commit_proposals:
            console.print("[yellow]No commit proposals to display.[/yellow]")
            return

        table = Table(title="Commit Proposals Summary")
        table.add_column("No.", style="dim", width=3)
        table.add_column("Proposed Message", style="cyan", no_wrap=False)
        table.add_column("Files", style="magenta")
        table.add_column("Explanation", style="yellow", no_wrap=False)

        for i, proposal in enumerate(self.result.commit_proposals):
            table.add_row(
                str(i + 1),
                proposal.commit_message,
                self._format_commit_proposal_changes(proposal.change_ids),
                proposal.explanation,
            )

        console.print(table)

        if (
            isinstance(self.result, IncompleteCommitProposalsResultSchema)
            and self.result.excluded_groups
        ):
            console.print()

            table = Table(title="Excluded Changes")
            table.add_column("Group No.", style="dim")
            table.add_column("Changes", style="magenta")
            table.add_column("Explanation", style="yellow", no_wrap=False, width=100)

            for i, group in enumerate(self.result.excluded_groups):
                if not group.change_ids:
                    continue

                table.add_row(
                    str(i + 1),
                    self._format_commit_proposal_changes(group.change_ids),
                    group.explanation,
                )

            console.print(table)

    def display_detailed_commit_proposals_summary(
        self,
    ):
        """Displays a detailed summary of the commit proposals in 'less' pager."""
        if not self.result or not self.result.commit_proposals:
            console.print("[yellow]No commit proposals to display.[/yellow]")
            return

        # Create content to display in less
        content = []

        for i, proposal in enumerate(self.result.commit_proposals):
            # Add a separator and proposal header with colors
            content.append("\033[1;36m" + "=" * 80 + "\033[0m")  # Bright cyan
            content.append(
                f"\033[1;32mCOMMIT PROPOSAL {i + 1} OF {len(self.result.commit_proposals)}\033[0m"
            )  # Bright green
            content.append("\033[1;36m" + "=" * 80 + "\033[0m")  # Bright cyan

            # Add commit message with color
            content.append(
                f"\033[1;33mCommit Message:\033[0m {proposal.commit_message}"
            )  # Yellow label
            content.append("")

            # Add explanation with color
            content.append(
                f"\033[1;35mExplanation:\033[0m {proposal.explanation}"
            )  # Magenta label
            content.append("")

            # Add separator before changes with color
            content.append("\033[1;34m" + "-" * 80 + "\033[0m")  # Blue
            content.append("\033[1;34mCHANGES:\033[0m")  # Blue
            content.append("\033[1;34m" + "-" * 80 + "\033[0m")  # Blue

            # Get and format all file diffs for this proposal
            file_diffs = self.ctx.get_file_diffs_from_change_ids(proposal.change_ids)

            for file_diff in file_diffs:
                # Get original git diff to preserve colors and formatting
                content.append(file_diff.original_diff)
                content.append("")

            # Add extra newline between proposals
            content.append("\n")

        # If there are excluded changes, show them too with colors
        if (
            isinstance(self.result, IncompleteCommitProposalsResultSchema)
            and self.result.excluded_groups
        ):
            content.append("\033[1;31m" + "=" * 80 + "\033[0m")  # Red
            content.append("\033[1;31mEXCLUDED CHANGES\033[0m")  # Red
            content.append("\033[1;31m" + "=" * 80 + "\033[0m")  # Red

            for i, group in enumerate(self.result.excluded_groups):
                if not group.change_ids:
                    continue

                content.append(f"\033[1;33mExcluded Group {i + 1}:\033[0m")  # Yellow
                content.append(
                    f"\033[1;35mExplanation:\033[0m {group.explanation}"
                )  # Magenta label
                content.append("")
                content.append("\033[1;31m" + "-" * 80 + "\033[0m")  # Red

                # Get and format all file diffs for this excluded group
                file_diffs = self.ctx.get_file_diffs_from_change_ids(group.change_ids)

                for file_diff in file_diffs:
                    content.append(file_diff.original_diff)
                    content.append("")

        # Write content to temporary file
        with tempfile.NamedTemporaryFile(mode="w", delete=False) as temp_file:
            temp_file.write("\n".join(content))
            temp_path = temp_file.name

        try:
            # Open less with the content
            subprocess.run(["less", "-R", temp_path])
        finally:
            # Clean up temporary file
            import os

            os.unlink(temp_path)

        self.prompt_main_workflow()

    def apply_all_commit_proposals(
        self,
    ):
        """Applies all commit proposals."""
        commit_proposals = self.result.commit_proposals
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
                self.ctx.stage_commit_proposal(proposal)
                console.print("[green]Changes staged successfully.[/green]")

                console.print("[cyan]Creating commit...[/cyan]")
                # In YOLO mode, commit directly without opening editor
                self.ctx.commit_commit_proposal(proposal)
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
                reset_staged_changes(self.repo)
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
                reset_staged_changes(self.repo)

                raise e

        self.result.commit_proposals = commit_proposals

    def apply_all_commit_proposals_and_push(self):
        """Applies all commit proposals and pushes to remote."""
        # First apply all commits
        self.apply_all_commit_proposals()

        # Check if any commits were actually applied
        if not self.result or not self.result.commit_proposals:
            console.print("[green]All proposals were applied successfully.[/green]")
        else:
            console.print(
                f"[yellow]Some proposals remain unapplied ({len(self.result.commit_proposals)} remaining).[/yellow]"
            )
            console.print(
                "[yellow]Skipping push due to incomplete commit application.[/yellow]"
            )
            return

        # Attempt to push
        console.print("[cyan]Pushing commits to remote...[/cyan]")

        try:
            # Get the current branch
            current_branch = self.repo.active_branch.name
            console.print(f"[cyan]Pushing branch '{current_branch}'...[/cyan]")

            # Push to remote
            origin = self.repo.remote("origin")
            push_result = origin.push(current_branch)

            if push_result:
                # Check if push was successful
                if any(
                    push.flags & (PushInfo.ERROR | PushInfo.REJECTED)
                    for push in push_result
                ):
                    console.print(
                        "[bold red]Push failed or was rejected by remote.[/bold red]"
                    )
                    for push_info in push_result:
                        if push_info.flags & PushInfo.ERROR:
                            console.print(f"[red]Error: {push_info.summary}[/red]")
                        elif push_info.flags & PushInfo.REJECTED:
                            console.print(f"[red]Rejected: {push_info.summary}[/red]")
                else:
                    console.print(
                        "[green]Successfully pushed commits to remote.[/green]"
                    )
            else:
                console.print(
                    "[yellow]Push completed, but no push information was returned.[/yellow]"
                )

        except git.GitCommandError as e:
            console.print(f"[bold red]Error during push: {e}[/bold red]")
            console.print(
                "[yellow]Commits were created locally but not pushed to remote.[/yellow]"
            )
        except ValueError as e:
            # This can happen if there's no remote named 'origin'
            console.print(f"[bold red]Error: {e}[/bold red]")
            console.print(
                "[yellow]No remote 'origin' found. Cannot push automatically.[/yellow]"
            )
            console.print(
                "[yellow]You can manually push with: git push origin <branch-name>[/yellow]"
            )
        except Exception as e:
            console.print(
                f"[bold red]An unexpected error occurred during push: {e}[/bold red]"
            )
            console.print(
                "[yellow]Commits were created locally but not pushed to remote.[/yellow]"
            )

    def display_final_summary(self):
        final_status = get_git_status(self.repo)
        if not final_status.changed_files and not final_status.untracked_files:
            # Check if *staged* changes exist from failed editor commit etc.
            if not has_staged_changes(self.repo):
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

    def display_detailed_commit_proposal(self, proposal: CommitProposalSchema):
        console.print(f"Detailed commit proposal: {proposal.commit_message}")
        console.print(f"Changes: {proposal.change_ids}")
        console.print(f"Explanation: {proposal.explanation}")

    def run_interactive_commit_workflow(self):
        console.print("\n[bold magenta]Entering Interactive Mode...[/bold magenta]")

        result_copy = deepcopy(self.result)
        total_proposals = len(result_copy.commit_proposals)
        committed_count = 0

        while result_copy.commit_proposals:
            proposal = result_copy.commit_proposals[0]

            current_num = total_proposals - len(result_copy.commit_proposals) + 1

            console.print("\n" + "=" * 40)
            console.print(f"[bold]Proposal {current_num} of {total_proposals}:[/bold]")

            self.display_detailed_commit_proposal(proposal)

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
                    self.ctx.stage_commit_proposal(proposal)
                    console.print("[green]Changes staged.[/green]")

                    console.print("[cyan]Opening editor for commit message...[/cyan]")
                    commit_successful = open_editor_for_commit(
                        self.repo, proposal.commit_message
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
                            reset_staged_changes(self.repo)
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
                        self.ctx.stage_commit_proposal(p)
                        console.print("[green]Changes staged successfully.[/green]")
                        console.print("[cyan]Creating commit...[/cyan]")
                        self.repo.index.commit(p.commit_message)  # Yolo -> No editor
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
                        reset_staged_changes(self.repo)
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
                        reset_staged_changes(self.repo)
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
                self.display_detailed_commit_proposals_summary()

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

    def prompt_main_workflow(self):
        choices = [
            ("Apply all proposed commits (#yolo)", "yolo"),
            ("Apply all proposed commits and push", "yolo-push"),
            (
                "Interactive: Review and commit each proposal one by one (opens editor)",
                "interactive",
            ),
            ("Show a detailed summary of all commit proposals", "summary"),
        ]

        if self.custom_instruction:
            choices.append(("Change custom instruction", "custom_instruction"))
        else:
            choices.append(("Use custom instruction", "custom_instruction"))

        choices.extend(
            [
                ("Rerun VibeGit (with current settings/instructions)", "rerun"),
                ("Quit: Exit without applying any proposals", "quit"),
            ]
        )

        questions = [
            inquirer.List(
                "mode",
                message="How do you want to proceed?",
                choices=choices,
                default="yolo",
            ),
        ]
        answers = inquirer.prompt(questions)
        mode = answers["mode"] if answers else "quit"

        if mode == "quit":
            console.print("[yellow]Exiting as requested.[/yellow]")
            sys.exit(0)
        if mode == "rerun":
            console.print("[yellow]Rerunning VibeGit with current settings...[/yellow]")
            self.run_commit_workflow()  # This will start a new flow
            return  # Exit current flow
        if mode == "custom_instruction":
            new_instruction_prompt = [
                inquirer.Text(
                    "new_instruction",
                    message="Enter new custom instruction (leave blank to clear)",
                    default=self.custom_instruction or "",
                )
            ]
            new_instruction_answer = inquirer.prompt(new_instruction_prompt)
            if new_instruction_answer:
                self.custom_instruction = (
                    new_instruction_answer["new_instruction"].strip() or None
                )
                if self.custom_instruction:
                    console.print(
                        f"[green]Custom instruction set to: '{self.custom_instruction}'[/green]"
                    )
                else:
                    console.print("[yellow]Custom instruction cleared.[/yellow]")
                console.print(
                    "[yellow]Rerunning VibeGit with new instruction...[/yellow]"
                )
                self.run_commit_workflow()  # This will start a new flow
                return  # Exit current flow
            else:  # User cancelled prompt
                self.prompt_main_workflow()  # Go back to main prompt
                return
        if mode == "summary":
            self.display_detailed_commit_proposals_summary()
            self.prompt_main_workflow()  # After summary, show prompt again
            return
        if mode == "yolo":
            self.apply_all_commit_proposals()
        elif mode == "yolo-push":
            self.apply_all_commit_proposals_and_push()
        elif mode == "interactive":
            self.run_interactive_commit_workflow()

    def run_commit_workflow(self):
        """Handles the main logic for the 'commit' subcommand."""
        console.print("[bold blue]VibeGit Commit Workflow Starting...[/bold blue]")

        # 1. Check for staged changes
        self.prepare_repo()

        # 2. Get Git Status and check for *any* changes
        status = self.get_git_status()

        # 3. Generate Commit Proposals
        self.ctx, self.result = self.generate_commit_proposals(status)

        self.display_commit_proposals_summary()

        self.prompt_main_workflow()

        self.display_final_summary()


def get_repo() -> git.Repo:
    try:
        repo = git.Repo(os.getcwd(), search_parent_directories=True)
        console.print(f"Found Git repository at: {repo.working_dir}")
        return repo
    except git.InvalidGitRepositoryError:
        console.print("[bold red]Error: Invalid Git repository detected.[/bold red]")
        sys.exit(1)
    except Exception as e:
        console.print(
            f"[bold red]Error initializing Git repository object: {e}[/bold red]"
        )
        sys.exit(1)


def run_commit(debug: bool = False, instruction: str | None = None):
    # For now, only the 'commit' subcommand is implemented directly.
    # Later, this could use argparse or Typer/Click to handle subcommands.
    # Example: if args.subcommand == 'commit': await run_commit_workflow()

    # Find Git repository
    repo = get_repo()

    try:
        cli = InteractiveCLI(config, repo, custom_instruction=instruction)
        cli.run_commit_workflow()
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
    """VibeGit - AI-powered Git commit assistant.

    VibeGit analyzes your uncommitted changes and intelligently groups them into
    semantic commits with AI-generated commit messages.

    Usage:
        vibegit [COMMAND] [OPTIONS]

    When run without a command, VibeGit will execute the commit workflow.
    It's recommended to explicitly use 'vibegit commit' instead.

    Examples:
        vibegit commit              # Analyze changes and create commits
        vibegit config              # View current configuration
        vibegit config wizard       # Run configuration wizard
    """
    if not ctx.invoked_subcommand:
        console.print(
            "[bold yellow]WARNING: If no command is provided, VibeGit will run the commit workflow. "
            "This is due to change. We recommend running VibeGit with the 'commit' command explicitly.[/bold yellow]"
        )
        run_commit()


@cli.command()
@click.option("--debug", "-d", is_flag=True, help="Enable debug mode")
@click.option(
    "--instruction",
    "-i",
    type=str,
    help="Custom instructions for commit generation (overrides .vibegitrules)",
)
def commit(debug: bool, instruction: str | None):
    """Analyze repository changes and create semantic commits.

    This command analyzes all uncommitted changes in your repository and:
    1. Groups related changes based on their semantic meaning
    2. Generates appropriate commit messages for each group
    3. Allows you to review and apply commits interactively or automatically

    The workflow supports three modes:
    - Interactive: Review and commit each proposal one by one (default)
    - YOLO: Automatically apply all proposed commits
    - Summary: View detailed breakdown of all proposals

    Options:
        --debug, -d         Enable debug mode with detailed error traces
        --instruction, -i   Custom instructions for commit generation
                           (overrides .vibegitrules file)

    Examples:
        vibegit commit
        vibegit commit --debug
        vibegit commit -i "Use conventional commit format"

    Note: VibeGit works best with unstaged changes. If you have staged
    changes, you'll be prompted to unstage them first.
    """
    run_commit(debug, instruction)


@cli.group(name="config", invoke_without_command=True)
@click.pass_context
def config_cli(ctx):
    """Manage VibeGit configuration.

    When run without a subcommand, displays the current configuration.

    Available subcommands:
        wizard  - Run interactive configuration wizard
        open    - Open config file in default editor
        path    - Show configuration file path
        get     - Get a specific configuration value
        set     - Set a configuration value

    Examples:
        vibegit config              # Show current configuration
        vibegit config wizard       # Run configuration wizard
        vibegit config get model.name
        vibegit config set model.name openai:gpt-4o
    """
    if not ctx.invoked_subcommand:
        pprint(config)


@config_cli.command()
def open():
    """Open the configuration file in your default editor.

    This command opens the VibeGit configuration file using your
    system's default application for .yaml files.

    Example:
        vibegit config open
    """
    import subprocess

    subprocess.run(["open", CONFIG_PATH])


@config_cli.command()
@click.argument("path", type=str)
def get(path: str):
    """Get a specific configuration value by path.

    PATH should be in dot notation (e.g., 'model.name').

    Arguments:
        PATH    Configuration path in dot notation

    Examples:
        vibegit config get model.name
        vibegit config get allow_excluding_changes
        vibegit config get watermark
    """
    pprint(config.get_by_path(path))


@config_cli.command()
@click.argument("path", type=str)
@click.argument("value", type=str)
def set(path: str, value: str):
    """Set a configuration value.

    PATH should be in dot notation (e.g., 'model.name').
    VALUE will be automatically converted to the appropriate type.

    Arguments:
        PATH    Configuration path in dot notation
        VALUE   New value to set

    Examples:
        vibegit config set model.name openai:gpt-4o
        vibegit config set allow_excluding_changes true
        vibegit config set watermark false

    Note: The configuration is immediately saved after setting the value.
    """
    config.set_by_path(path, value)
    config.save_config()


@config_cli.command()
def path():
    """Display the path to the configuration file.

    This shows the full filesystem path where VibeGit stores its
    configuration. Useful for manual editing or backup.

    Example:
        vibegit config path
    """
    print(CONFIG_PATH)


@config_cli.command()
def wizard():
    """Run the configuration wizard to set up VibeGit interactively.

    This command launches an interactive configuration wizard that helps you:
    - Choose an LLM model (Gemini, GPT, or custom)
    - Configure API keys for your chosen model
    - Save settings to the configuration file

    The wizard can be run at any time, regardless of whether a config
    file already exists. Use this to reconfigure VibeGit or change
    your model settings.

    Example:
        vibegit config wizard

    Note: The wizard will guide you through each step interactively.
    """
    from vibegit.wizard import ConfigWizard

    wizard = ConfigWizard()
    wizard.run()


if __name__ == "__main__":
    cli()
