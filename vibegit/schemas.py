from pydantic import BaseModel, Field


class CommitProposal(BaseModel):
    reasoning: str = Field(
        description="A reasoning for the decision of grouping these changes together."
    )
    commit_message: str = Field(description="The proposed commit message")
    change_ids: list[int] = Field(
        description="A list of changes (hunks or files) that should go into this commit. Use the provided change IDs (only the number)."
    )


class CommitGroupingProposal(BaseModel):
    commit_proposals: list[CommitProposal] = Field(
        description="A list of commit proposals"
    )
