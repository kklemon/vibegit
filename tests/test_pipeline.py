import pytest
from conftest import test_repositories, test_models

from vibegit.ai import CommitProposalAI
from vibegit.git import CommitProposalContext, GitContextFormatter, get_git_status


@pytest.mark.parametrize("repo", list(test_repositories), indirect=True)
@pytest.mark.parametrize("chat_model", test_models, indirect=True)
def test_pipeline(repo, chat_model):
    formatter = GitContextFormatter(
        truncate_lines=120,
        include_latest_commits=5,
    )
    status = get_git_status(repo)
    ctx = CommitProposalContext(git_status=status)
    context = formatter.format_changes(ctx)
    ai = CommitProposalAI(
        chat_model,
        allow_excluding_changes=False,
    )

    result = ai.propose_commits(context)

    ctx.validate_commit_proposal(result)

    assert result is not None
    assert len(result.commit_proposals) > 0

    num_commits_before = len(list(repo.iter_commits()))

    for proposal in result.commit_proposals:
        assert bool(proposal.commit_message)
        assert len(proposal.change_ids) > 0

        ctx.stage_commit_proposal(proposal)
        ctx.commit_commit_proposal(proposal)

    num_commits_after = len(list(repo.iter_commits()))

    assert num_commits_after == num_commits_before + len(result.commit_proposals)

    status = get_git_status(repo)

    assert status.changed_files == []
    assert status.untracked_files == []
