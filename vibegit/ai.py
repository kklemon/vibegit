from typing import cast
from langchain_core.language_models.chat_models import BaseChatModel

from vibegit.prompts import build_system_prompt
from vibegit.schemas import CommitProposalListSchema, IncompleteCommitProposalListSchema


class CommitProposalAI:
    def __init__(self, model: BaseChatModel, allow_excluding_changes: bool = False):
        schema = (
            IncompleteCommitProposalListSchema
            if allow_excluding_changes
            else CommitProposalListSchema
        )
        self.model = model.with_structured_output(schema)
        self.allow_excluding_changes = allow_excluding_changes

    async def propose_commits(
        self, context: str
    ) -> CommitProposalListSchema | IncompleteCommitProposalListSchema | None:
        result = await self.model.ainvoke(
            [
                {
                    "role": "system",
                    "content": build_system_prompt(self.allow_excluding_changes),
                },
                {"role": "user", "content": context},
            ]
        )

        if result is None:
            return None

        if self.allow_excluding_changes:
            return cast(IncompleteCommitProposalListSchema, result)
        else:
            return cast(CommitProposalListSchema, result)
