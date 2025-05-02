import abc
from copy import deepcopy
from dataclasses import dataclass, field
from itertools import islice
import tempfile

import git
from unidiff import Hunk, PatchedFile, PatchSet
from vibegit.schemas import CommitGroupingProposal, CommitProposal


@dataclass
class FileDiff:
    patched_file: PatchedFile
    original_diff: str

    def __hash__(self):
        return hash(self.original_diff)


@dataclass
class BaseGitFile:
    repo: git.Repo
    filename: str

    @abc.abstractmethod
    def get_diff(self) -> FileDiff:
        pass


class ChangedGitFile(BaseGitFile):
    def get_diff(self) -> FileDiff:
        output = self.repo.git.diff("--no-color", "--binary", "--", self.filename)
        patch_set = PatchSet(output)
        assert len(patch_set) == 1
        return FileDiff(patch_set[0], output)


class UntrackedGitFile(BaseGitFile):
    def get_diff(self) -> FileDiff:
        output = self.repo.git.diff(
            "--no-color",
            "--no-index",
            "--binary",
            "--",
            "/dev/null",
            self.filename,
            with_exceptions=False,
        )
        patch_set = PatchSet(output)
        assert len(patch_set) == 1
        return FileDiff(patch_set[0], output)


@dataclass
class GitStatusSummary:
    repo: git.Repo
    changed_files: list[ChangedGitFile]
    untracked_files: list[UntrackedGitFile]


def get_git_status(repo) -> GitStatusSummary:
    # --- Get Untracked Files ---
    # repo.untracked_files provides a list of files Git doesn't track
    untracked_files = [
        UntrackedGitFile(repo, filename) for filename in repo.untracked_files
    ]

    # --- Get Changed Files (Staged and Unstaged) ---
    changed_files = []  # Using a set to avoid duplicates

    # Compare the index (staging area) with the HEAD commit for staged changes
    # This includes Added (A), Deleted (D), Modified (M), Renamed (R), Copied (C), Type Changed (T)
    staged_diffs = repo.index.diff("HEAD")
    for diff_obj in staged_diffs:
        # diff_obj.a_path is the path in HEAD, diff_obj.b_path is the path in the index
        # For new files (A), a_path is None. For deleted files (D), b_path is None.
        path = diff_obj.b_path if diff_obj.change_type == "A" else diff_obj.a_path
        if path:
            changed_files.append(ChangedGitFile(repo, path))

    # Compare the working directory with the index for unstaged changes
    unstaged_diffs = repo.index.diff(None)
    for diff_obj in unstaged_diffs:
        # diff_obj.a_path is the path in the index, diff_obj.b_path is the path in the worktree
        # For deleted files (D), b_path is None. For modified (M), they are the same.
        # Untracked files are not typically listed here but handled separately.
        path = diff_obj.a_path
        if path:
            changed_files.append(ChangedGitFile(repo, path))

    return GitStatusSummary(
        repo=repo, changed_files=list(changed_files), untracked_files=untracked_files
    )


@dataclass
class HunkFileReference:
    file: FileDiff
    hunk: Hunk


@dataclass
class CommitProposalContext:
    git_status: GitStatusSummary
    hunk_counter: int = 0
    hunk_id_to_hunk: dict[int, HunkFileReference] = field(default_factory=dict)

    def validate_commit_proposal(self, commit_proposals: CommitGroupingProposal):
        # First, verify that all referenced hunks exist
        for proposal in commit_proposals.commit_proposals:
            for hunk_id in proposal.hunk_ids:
                if hunk_id not in self.hunk_id_to_hunk:
                    raise ValueError(
                        f"Hunk ID {hunk_id} not found in formatting context"
                    )

        # Then verify that each hunk is only included in one commit proposal
        hunk_ids = set()
        for proposal in commit_proposals.commit_proposals:
            for hunk_id in proposal.hunk_ids:
                if hunk_id in hunk_ids:
                    raise ValueError(
                        f"Hunk ID {hunk_id} is included in multiple commit proposals"
                    )
                hunk_ids.add(hunk_id)

    def stage_commit_proposal(self, commit_proposal: CommitProposal):
        # Dereference the hunks
        hunk_file_refs = [
            self.hunk_id_to_hunk[hunk_id] for hunk_id in commit_proposal.hunk_ids
        ]

        # Group by file
        hunk_file_refs_by_file: dict[FileDiff, list[HunkFileReference]] = {}

        for hunk_file_ref in hunk_file_refs:
            if hunk_file_ref.file.patched_file.is_binary_file:
                assert hunk_file_refs_by_file.get(hunk_file_ref.file) is None, (
                    "Found more than one change for a binary file but expected only one"
                )
                hunk_file_refs_by_file[hunk_file_ref.file] = [hunk_file_ref]
            else:
                hunk_file_refs_by_file.setdefault(hunk_file_ref.file, []).append(
                    hunk_file_ref
                )

        # Create a new file with the hunks of each group
        files = []

        for file_diff, group in hunk_file_refs_by_file.items():
            if file_diff.patched_file.is_binary_file:
                assert len(group) == 1, (
                    "Binary files should only have one change by our logic"
                )
                files.append(file_diff.original_diff)
                continue

            assert len(group) > 0

            patched_file = deepcopy(group[0].file.patched_file)
            patched_file.clear()

            for hunk_file_ref in group:
                if hunk_file_ref.hunk:
                    # Ensure the hunk ends with a newline
                    patched_file.append(hunk_file_ref.hunk)
            files.append(patched_file)

        # Stage the files
        for patched_file in files:
            with tempfile.NamedTemporaryFile(delete=False, buffering=0) as f:
                f.write((str(patched_file).strip() + "\n\n\n").encode("utf-8"))
                f.flush()

                try:
                    self.git_status.repo.git.execute(
                        ["git", "apply", "--cached", f.name]
                    )
                except Exception as e:
                    print(f"Error staging file: {patched_file}. Patch file: {f.name}")
                    raise e

    def commit_commit_proposal(self, commit_proposal: CommitProposal):
        self.git_status.repo.git.commit("-m", commit_proposal.commit_message)


class GitContextFormatter:
    def __init__(
        self,
        include_active_branch: bool = True,
        truncate_lines: int | None = None,
        include_latest_commits: int | None = 5,
        changes_last: bool = False,
        user_instructions: str | None = None,
    ):
        self.include_active_branch = include_active_branch
        self.include_latest_commits = include_latest_commits
        self.truncate_lines = truncate_lines
        self.changes_last = changes_last
        self.user_instructions = user_instructions

    def _truncate_line(self, line: str) -> str:
        truncated_line = line[: self.truncate_lines]
        if self.truncate_lines and len(line) > self.truncate_lines:
            truncated_line += "..."
        return truncated_line

    def _format_file(self, file: BaseGitFile, ctx: CommitProposalContext):
        diff = file.get_diff()

        if diff.patched_file.is_binary_file:
            # If it's a binary file, add the hunk id to the first line
            # and use a placeholder for the content
            hunk_id = ctx.hunk_counter

            patch_info = diff.patched_file.patch_info
            assert patch_info is not None, "Expected patch info but found None"
            patch_info_lines = str(patch_info).splitlines()
            patch_info_lines[0] = (
                f"{patch_info_lines[0]}  # Hunk ID: {ctx.hunk_counter}"
            )

            ctx.hunk_counter += 1
            ctx.hunk_id_to_hunk[hunk_id] = HunkFileReference(
                diff, None
            )

            result = "\n".join(
                [
                    "\n".join(patch_info_lines).strip(),
                    "<binary data placeholder>",
                ]
            )

            return result

        for hunk in diff.patched_file:
            hunk_id = ctx.hunk_counter
            hunk.section_header = f"  # Hunk ID: {hunk_id}"
            ctx.hunk_counter += 1
            ctx.hunk_id_to_hunk[hunk_id] = HunkFileReference(diff, hunk)

        formatted_diff_lines = str(diff.patched_file).splitlines()

        if not len(diff.patched_file):
            hunk_id = ctx.hunk_counter
            formatted_diff_lines[0] = (
                formatted_diff_lines[0] + f"  # Hunk ID: {hunk_id}"
            )
            ctx.hunk_counter += 1
            ctx.hunk_id_to_hunk[hunk_id] = HunkFileReference(diff, None)

        if self.truncate_lines:
            formatted_diff_lines = [
                self._truncate_line(line) for line in formatted_diff_lines
            ]

        return "\n".join(formatted_diff_lines)

    def _get_latest_commits(self, repo: git.Repo):
        return list(islice(repo.iter_commits(), self.include_latest_commits))

    def _format_commit_message(self, message: str):
        lines = message.splitlines()
        message = "\n".join("   " * bool(i) + line for i, line in enumerate(lines))
        return f'"{message}"'

    def format_changes(self, ctx: CommitProposalContext) -> str:
        output_parts = []

        formatted_changed_files = [
            self._format_file(file, ctx) for file in ctx.git_status.changed_files
        ]
        formatted_untracked_files = [
            self._format_file(file, ctx) for file in ctx.git_status.untracked_files
        ]

        def add_file_changes():
            output_parts.append("Changed Files:")
            output_parts.append("\n\n".join(formatted_changed_files))
            output_parts.append("Untracked Files:")
            output_parts.append("\n\n".join(formatted_untracked_files))

        if not self.changes_last:
            add_file_changes()

        if self.include_active_branch:
            output_parts.append(
                f"Active Branch: {ctx.git_status.repo.active_branch.name}"
            )

        if self.include_latest_commits:
            latest_commits = self._get_latest_commits(ctx.git_status.repo)
            parts = ["Latest Commits:"]
            for i, commit in enumerate(latest_commits):
                parts.append(
                    f"{i + 1}. {self._format_commit_message(commit.message.strip())}"
                )
            output_parts.append("\n".join(parts))

        if self.user_instructions:
            output_parts.append(f"User Instructions: {self.user_instructions}")

        if self.changes_last:
            add_file_changes()

        return "\n\n".join(output_parts)
