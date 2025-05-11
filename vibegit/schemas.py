from pydantic import BaseModel, Field


class CommitProposalSchema(BaseModel):
    explanation: str = Field(
        description="An explanation for the decision of grouping these changes together."
    )
    commit_message: str = Field(description="The proposed commit message")
    change_ids: list[int] = Field(
        description="A list of changes (hunks or files) that should go into this commit. Use the provided change IDs (only the number)."
    )


class CommitProposalsResultSchema(BaseModel):
    commit_proposals: list[CommitProposalSchema] = Field(
        description="A list of commit proposals"
    )


class ExcludeChangesSchema(BaseModel):
    explanation: str = Field(
        description="An explanation for the decision of excluding these changes."
    )
    change_ids: list[int] = Field(
        description="A list of change IDs that should not be included in any commit as they may be incomplete or not ready to be committed."
    )


class IncompleteCommitProposalsResultSchema(CommitProposalsResultSchema):
    excluded_groups: list[ExcludeChangesSchema] = Field(
        default_factory=list,
        description="Groups of changes that were excluded from the commit proposals.",
    )
