from typing import cast
from langchain_core.language_models.chat_models import BaseChatModel

from vibegit.git import CommitProposalContext, GitContextFormatter
from vibegit.prompts import COMMIT_PROPOSAL_SYSTEM_PROMPT
from vibegit.schemas import CommitGroupingProposal


class CommitProposalAI:
    def __init__(self, model: BaseChatModel):
        self.model = model.with_structured_output(CommitGroupingProposal)

    async def propose_commits(self, context: str) -> CommitGroupingProposal | None:
        result = await self.model.ainvoke(
            [
                {"role": "system", "content": COMMIT_PROPOSAL_SYSTEM_PROMPT},
                {"role": "user", "content": context},
            ]
        )

        if result is None:
            return None

        return cast(CommitGroupingProposal, result)
