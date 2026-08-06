from datetime import datetime
import json

from pydantic import BaseModel, ConfigDict, field_validator


class SourceOut(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    connector: str
    name: str
    enabled: bool
    last_synced_at: datetime | None
    last_error: str | None
    error_count: int
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
    window_text: str = ""
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


class AskTurn(BaseModel):
    role: str  # user | assistant
    content: str


class AskRequest(BaseModel):
    question: str
    history: list[AskTurn] = []  # previous turns, oldest first


class Citation(BaseModel):
    entity_id: int | None
    kind: str
    summary: str
    source_ref: str
    score: float
    snippet: str


class AskResponse(BaseModel):
    answer: str
    citations: list[Citation]


class SystemEventOut(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    component: str
    level: str
    source_id: int | None
    source_name: str | None = None
    message: str
    detail: str
    created_at: datetime


class LogFileOut(BaseModel):
    name: str
    size: int
    modified: datetime


class JobOut(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    source_id: int
    kind: str
    status: str  # running | done | failed
    total: int
    processed: int
    result: dict = {}
    error: str | None
    created_at: datetime
    started_at: datetime | None
    finished_at: datetime | None

    @field_validator("result", mode="before")
    @classmethod
    def _parse_result(cls, value):
        if isinstance(value, str):
            return json.loads(value or "{}")
        return value
