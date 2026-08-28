from types import SimpleNamespace

from vibegit.ai import CommitProposalAI
from vibegit.schemas import CommitProposalSchema, CommitProposalsResultSchema


def test_propose_commits_returns_current_pydantic_ai_output():
    expected = CommitProposalsResultSchema(
        commit_proposals=[
            CommitProposalSchema(
                explanation="Related changes",
                commit_message="Update files",
                change_ids=[1],
            )
        ]
    )
    ai = CommitProposalAI.__new__(CommitProposalAI)
    ai.allow_excluding_changes = False
    ai._agent = SimpleNamespace(
        run_sync=lambda _context: SimpleNamespace(output=expected)
    )

    assert ai.propose_commits("context") is expected
