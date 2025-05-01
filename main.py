import time
import sys
from typing import List, Optional

# --- Pydantic Models (for structure, Optional dependency) ---
# If you don't have pydantic, you can replace these with simple dicts
# or dataclasses, but Pydantic helps enforce the schema.
try:
    from pydantic import BaseModel, Field
except ImportError:
    print("Pydantic not found, using basic classes (validation disabled).")
    print("Install with: pip install pydantic")

    class BaseModel:
        pass # Basic placeholder

    def Field(*, description):
        return None # Placeholder

class CommitProposal(BaseModel):
    commit_description: str = Field(
        description="A loose description for this commit. This will NOT end up in the commit message and is just for internal purposes."
    )
    commit_message: str = Field(
        description="The proposed commit message"
    )
    hunk_ids: List[str] = Field(description="A list of hunks that should go into this commit. Use the provided hunk IDs.")

class CommitGroupingProposal(BaseModel):
    commit_proposal: List[CommitProposal] = Field(
        description="A list of commit proposals"
    )

# --- Stub Functions (Simulating Git and LLM) ---

def get_git_diff_with_hunk_ids() -> str:
    """Simulates running `git diff` and adding unique hunk IDs."""
    print("[italic gray]Simulating: Reading git diff...[/italic gray]")
    # Simulate some delay
    time.sleep(0.2)
    return """
diff --git a/src/component_a.py b/src/component_a.py
index 111..222 100644
--- a/src/component_a.py
+++ b/src/component_a.py
@@ -10,6 +10,7 @@
 class ComponentA:
     def __init__(self):
         self.name = "Component A"
+        self.status = "initialized" # Hunk ID: 1

     def run(self):
         print(f"Running {self.name}")
diff --git a/src/utils.py b/src/utils.py
index 333..444 100644
--- a/src/utils.py
+++ b/src/utils.py
@@ -1,4 +1,5 @@
 # Utility functions
 def helper_function():
-    pass # Hunk ID: 2
+    # Implement actual logic later
+    print("Helper executed") # Hunk ID: 3

diff --git a/tests/test_component_a.py b/tests/test_component_a.py
new file mode 100644
index 000..555
--- /dev/null
+++ b/tests/test_component_a.py
@@ -0,0 +1,7 @@
+import unittest # Hunk ID: 4
+from src.component_a import ComponentA
+
+class TestComponentA(unittest.TestCase):
+    def test_init(self):
+        comp = ComponentA()
+        self.assertEqual(comp.status, "initialized") # Hunk ID: 5
"""

def get_current_branch() -> str:
    """Simulates getting the current git branch."""
    print("[italic gray]Simulating: Getting current branch...[/italic gray]")
    time.sleep(0.1)
    return "feat/add-component-status"

def get_recent_commits() -> List[str]:
    """Simulates getting recent commit messages."""
    print("[italic gray]Simulating: Getting recent commits...[/italic gray]")
    time.sleep(0.1)
    return [
        "feat: Add initial structure for ComponentA",
        "refactor: Move utility function",
        "ci: Setup basic pipeline",
    ]

def get_commit_proposals(diff: str, branch: str, history: List[str]) -> CommitGroupingProposal:
    """
    Simulates the LLM processing the context and returning commit proposals.
    This function would contain the actual call to your LLM in a real scenario.
    """
    print("[italic blue]Simulating: Analyzing changes and generating proposals (LLM)...[/italic blue]")
    # Based on the fake diff and context:
    # - Hunks 1 & 5 relate to the new status feature in ComponentA and its test.
    # - Hunk 4 is adding unittest import for the new test file. Often grouped with test.
    # - Hunks 2 & 3 are changes in utils.py (removing pass, adding print).
    # The branch name `feat/add-component-status` suggests grouping #1, #4, #5.
    # The history shows `feat:` prefix is used.
    time.sleep(0.5)

    proposal1 = CommitProposal(
        commit_description="Adds the status field to ComponentA and includes the corresponding test update and necessary import.",
        commit_message="feat: Add status field and test for ComponentA",
        hunk_ids=["1", "4", "5"]
    )

    proposal2 = CommitProposal(
        commit_description="Updates the helper function in utils.py, replacing placeholder.",
        commit_message="refactor: Update placeholder in helper_function",
        hunk_ids=["2", "3"] # Note: Git diff shows removing hunk 2 and adding hunk 3
                            # A real system needs to handle hunk mapping carefully.
                            # For this stub, we just list IDs mentioned in the diff section.
    )

    return CommitGroupingProposal(commit_proposal=[proposal1, proposal2])


def stage_hunks(hunk_ids: List[str]) -> bool:
    """Simulates staging specific hunks using `git add --patch` or similar."""
    print(f"[italic green]Simulating: Staging hunks {hunk_ids}...[/italic green]")
    # In reality, this would involve parsing the diff, potentially writing
    # temporary patch files, and applying them selectively.
    time.sleep(0.3)
    print(f"[green]Success: Staged hunks {hunk_ids}.[/green]")
    return True # Simulate success

def create_commit(message: str) -> bool:
    """Simulates creating a commit."""
    print(f"[italic green]Simulating: Committing with message...[/italic green]")
    # Escape quotes for display if necessary
    display_message = message.replace('"', '\\"')
    print(f'> git commit -m "{display_message}"')
    time.sleep(0.2)
    print(f"[green]Success: Created commit.[/green]")
    return True # Simulate success

# --- CLI Logic ---

def display_summary(proposals: List[CommitProposal]):
    """Displays a summary of the remaining commit proposals."""
    if not proposals:
        console.print("[yellow]No remaining proposals.[/yellow]")
        return

    from rich.table import Table

    table = Table(title="Remaining Commit Proposals Summary")
    table.add_column("No.", style="dim", width=3)
    table.add_column("Proposed Message", style="cyan", no_wrap=False)
    table.add_column("Hunks", style="magenta")
    table.add_column("Internal Description", style="yellow", no_wrap=False)


    for i, proposal in enumerate(proposals):
        table.add_row(
            str(i + 1),
            proposal.commit_message,
            ", ".join(proposal.hunk_ids),
            proposal.commit_description,
        )

    console.print(table)


def run_vibecommit():
    """Main function for the CLI tool."""
    console.print("[bold blue]vibecommit - AI Semantic Commit Grouper[/bold blue]")
    console.print("-" * 40)

    # 1. Scan Repo (Stubs)
    diff = get_git_diff_with_hunk_ids()
    branch = get_current_branch()
    history = get_recent_commits()

    # Basic check if there's anything to process
    if not diff.strip():
      console.print("[yellow]No changes detected in the repository.[/yellow]")
      return

    # 2. Get Proposals (Stub)
    grouping_proposal = get_commit_proposals(diff, branch, history)
    proposals = grouping_proposal.commit_proposal

    if not proposals:
        console.print("[yellow]No commit proposals generated.[/yellow]")
        return

    console.print(f"[green]Generated {len(proposals)} commit proposal(s).[/green]")

    # 3. Interactive Loop
    while proposals:
        console.print("-" * 40)
        next_proposal = proposals[0]
        remaining_count = len(proposals)

        console.print(f"[bold]Next Proposed Commit ({remaining_count} remaining):[/bold]")
        console.print(f"  [cyan]Message:[/cyan] {next_proposal.commit_message}")
        console.print(f"  [magenta]Hunks:[/magenta] {', '.join(next_proposal.hunk_ids)}")
        console.print(f"  [yellow]Description:[/yellow] {next_proposal.commit_description}")
        console.print("-" * 40)

        console.print("[bold]Choose an action:[/bold]")
        console.print("  [green](N)ext[/green]: Stage and commit this proposal.")
        console.print("  [blue](A)ll[/blue]: Stage and commit ALL remaining proposals.")
        console.print("  [yellow](S)ummary[/yellow]: View summary of all remaining proposals.")
        # Add Skip? Diff view? Re-generate? - Future enhancements
        console.print("  [red](Q)uit[/red]: Exit without committing remaining proposals.")

        choice = input("Your choice (N/A/S/Q): ").strip().upper()

        if choice == 'N':
            console.print(f"\nApplying proposal 1 of {remaining_count}...")
            if stage_hunks(next_proposal.hunk_ids):
                if create_commit(next_proposal.commit_message):
                    proposals.pop(0) # Remove applied proposal
                    console.print("[bold green]Proposal applied successfully.[/bold green]")
                else:
                    console.print("[bold red]Failed to create commit. Stopping.[/bold red]")
                    break # Stop if commit fails
            else:
                console.print("[bold red]Failed to stage hunks. Stopping.[/bold red]")
                break # Stop if staging fails

        elif choice == 'A':
            console.print(f"\nApplying ALL {remaining_count} remaining proposals...")
            all_successful = True
            original_count = remaining_count
            for i in range(original_count):
                proposal_to_apply = proposals[0] # Always apply the next one
                console.print(f"\nApplying proposal {i+1} of {original_count}: '{proposal_to_apply.commit_message}'")
                if stage_hunks(proposal_to_apply.hunk_ids):
                    if create_commit(proposal_to_apply.commit_message):
                        proposals.pop(0)
                    else:
                        console.print(f"[bold red]Failed to create commit for proposal {i+1}. Stopping.[/bold red]")
                        all_successful = False
                        break
                else:
                    console.print(f"[bold red]Failed to stage hunks for proposal {i+1}. Stopping.[/bold red]")
                    all_successful = False
                    break

            if all_successful:
                 console.print("[bold green]All proposals applied successfully.[/bold green]")
            else:
                 console.print("[bold yellow]Finished applying proposals (with errors).[/bold yellow]")
            break # Exit loop after attempting all

        elif choice == 'S':
            display_summary(proposals)
            # Loop continues to show the menu again

        elif choice == 'Q':
            console.print("[yellow]Quitting. No further commits were made.[/yellow]")
            break

        else:
            console.print("[red]Invalid choice. Please try again.[/red]")

    # End of loop
    if not proposals:
        console.print("\n[bold green]All proposed commits have been applied.[/bold green]")
    console.print("\nvibecommit finished.")


if __name__ == "__main__":
    try:
        from rich.console import Console
        console = Console()
    except ImportError:
        print("Rich library not found. Output formatting will be basic.")
        print("Install with: pip install rich")
        # Basic print fallback
        class BasicConsole:
            def print(self, text, *args, **kwargs):
                # Basic filtering of rich tags for non-rich consoles
                import re
                text = re.sub(r'\[/?.*?\]', '', text)
                print(text)
        console = BasicConsole()

    run_vibecommit()