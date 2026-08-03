from datetime import datetime

from pydantic import BaseModel, ConfigDict


class SourceOut(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    connector: str
    name: str
    enabled: bool
    last_synced_at: datetime | None
    created_at: datetime


class SourceCreate(BaseModel):
    connector: str
    name: str
    config: dict = {}


class EntityOut(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    item_id: int
    kind: str
    summary: str
    reasoning: str
    confidence: float
    author: str
    source_ref: str
    status: str
    owner: str
    created_at: datetime
    updated_at: datetime


class EntityUpdate(BaseModel):
    kind: str | None = None
    status: str | None = None
    owner: str | None = None


class MergeProposal(BaseModel):
    id: int
    entity_a_id: int
    entity_b_id: int
    reason: str = "similar summaries"


class MergeDecision(BaseModel):
    proposal_id: int
    decision: str  # merge | dismiss


class AskRequest(BaseModel):
    question: str


class Citation(BaseModel):
    entity_id: int
    kind: str
    summary: str
    source_ref: str
    score: float
    snippet: str


class AskResponse(BaseModel):
    answer: str
    citations: list[Citation]
