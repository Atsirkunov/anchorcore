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


class SourceDetailOut(SourceOut):
    """B14: source + resolved config with secrets masked (MCP get_source)."""

    config: dict = {}


class SourceCreate(BaseModel):
    connector: str
    name: str
    config: dict = {}


class SourceUpdate(BaseModel):
    """B7: editable source fields. Omitted fields are left unchanged; secret
    config values sent as the '***set***' placeholder keep the stored secret."""

    name: str | None = None
    enabled: bool | None = None
    config: dict | None = None


class ProjectCreate(BaseModel):
    name: str
    source_ids: list[int] = []


class ProjectOut(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    name: str
    is_default: bool
    created_at: datetime
    source_ids: list[int] = []


class ProjectUpdate(BaseModel):
    name: str | None = None
    is_default: bool | None = None
    source_ids: list[int] | None = None


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
    dispute_count: int = 0
    created_at: datetime
    updated_at: datetime


class EntityUpdate(BaseModel):
    kind: str | None = None
    status: str | None = None
    owner: str | None = None


class DisputeCreate(BaseModel):
    reason: str = ""
    user: str = ""


class DisputeOut(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    entity_id: int
    reason: str
    user: str
    created_at: datetime


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
    project_id: int | None = None  # B15: scope retrieval to a project's sources


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


class SearchRequest(BaseModel):
    """B14: raw retrieval for agents/harnesses — ranked context, not a final
    answer. `k` caps the number of hits; `project_id` scopes retrieval (B15)."""

    query: str
    k: int = 8
    project_id: int | None = None


class SearchHit(BaseModel):
    chunk_id: int | None
    entity_id: int | None
    kind: str  # decision|document|action|note — or "document" for raw chunks
    summary: str
    content: str
    source_ref: str
    source_id: int | None
    item_id: int | None
    item_title: str
    score: float


class SearchResponse(BaseModel):
    hits: list[SearchHit]


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
