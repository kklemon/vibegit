from typing import Any, cast

from pydantic_ai import Agent

from vibegit.prompts import build_system_prompt
from vibegit.schemas import (
    CommitProposalsResultSchema,
    IncompleteCommitProposalsResultSchema,
)


class CommitProposalAI:
    def __init__(
        self,
        model: Any,
        allow_excluding_changes: bool = False,
        model_settings: Any | None = None,
    ):
        schema: type[
            CommitProposalsResultSchema | IncompleteCommitProposalsResultSchema
        ] = (
            IncompleteCommitProposalsResultSchema
            if allow_excluding_changes
            else CommitProposalsResultSchema
        )
        self.allow_excluding_changes = allow_excluding_changes
        kwargs: dict[str, Any] = {
            "model": model,
            "system_prompt": build_system_prompt(self.allow_excluding_changes),
            "output_type": schema,
        }
        if model_settings is not None:
            kwargs["model_settings"] = model_settings
        self._agent = Agent(**kwargs)

    def propose_commits(
        self, context: str
    ) -> CommitProposalsResultSchema | IncompleteCommitProposalsResultSchema | None:
        result = self._agent.run_sync(context)

        if hasattr(result, "output"):
            result = result.output
        elif hasattr(result, "data"):
            result = result.data

        if result is None:
            return None

        if self.allow_excluding_changes:
            return cast(IncompleteCommitProposalsResultSchema, result)
        else:
            return cast(CommitProposalsResultSchema, result)
