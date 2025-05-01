from pydantic import BaseModel, Field


class CommitProposal(BaseModel):
    reasoning: str = Field(
        description="A reasoning for the decision behind grouping these hunks together."
    )
    commit_message: str = Field(description="The proposed commit message")
    hunk_ids: list[int] = Field(
        description="A list of hunks that should go into this commit. Use the provided hunk IDs (only the number)."
    )


class CommitGroupingProposal(BaseModel):
    commit_proposals: list[CommitProposal] = Field(
        description="A list of commit proposals"
    )
