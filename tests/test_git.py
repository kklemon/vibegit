import git
import pytest

from vibegit.git import CommitProposalContext, GitStatusSummary, get_git_status
from vibegit.schemas import (
    CommitProposalSchema,
    CommitProposalsResultSchema,
    ExcludeChangesSchema,
    IncompleteCommitProposalsResultSchema,
)


def make_repo(tmp_path) -> git.Repo:
    repo = git.Repo.init(tmp_path)
    tracked_file = tmp_path / "tracked.txt"
    tracked_file.write_text("original\n")
    repo.index.add(["tracked.txt"])
    repo.index.commit("initial commit")
    return repo


def make_context(*change_ids: int) -> CommitProposalContext:
    context = CommitProposalContext(
        GitStatusSummary(repo=None, changed_files=[], untracked_files=[])
    )
    context.change_id_to_ref = dict.fromkeys(change_ids, None)
    return context


def proposal(*change_ids: int) -> CommitProposalSchema:
    return CommitProposalSchema(
        explanation="Related changes",
        commit_message="Update files",
        change_ids=list(change_ids),
    )


def test_git_status_combines_staged_and_unstaged_changes(tmp_path):
    repo = make_repo(tmp_path)
    tracked_file = tmp_path / "tracked.txt"

    tracked_file.write_text("original\nstaged\n")
    repo.index.add(["tracked.txt"])
    tracked_file.write_text("original\nstaged\nunstaged\n")
    (tmp_path / "untracked.txt").write_text("new\n")

    status = get_git_status(repo)

    assert [file.filename for file in status.changed_files] == ["tracked.txt"]
    assert [file.filename for file in status.untracked_files] == ["untracked.txt"]
    diff = status.changed_files[0].get_diff().original_diff
    assert "+staged" in diff
    assert "+unstaged" in diff


def test_validate_commit_proposal_requires_every_change():
    context = make_context(1, 2)
    result = CommitProposalsResultSchema(commit_proposals=[proposal(1)])

    with pytest.raises(ValueError, match="Changes not covered.*2"):
        context.validate_commit_proposal(result)


def test_validate_commit_proposal_counts_excluded_changes_as_covered():
    context = make_context(1, 2)
    result = IncompleteCommitProposalsResultSchema(
        commit_proposals=[proposal(1)],
        excluded_groups=[ExcludeChangesSchema(explanation="Not ready", change_ids=[2])],
    )

    context.validate_commit_proposal(result)


def test_validate_commit_proposal_rejects_duplicates_across_exclusions():
    context = make_context(1)
    result = IncompleteCommitProposalsResultSchema(
        commit_proposals=[proposal(1)],
        excluded_groups=[ExcludeChangesSchema(explanation="Not ready", change_ids=[1])],
    )

    with pytest.raises(ValueError, match="multiple proposal or exclusion groups"):
        context.validate_commit_proposal(result)
