from datetime import datetime

from pydantic import BaseModel, ConfigDict


class KnowledgeObjectOut(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    document_id: int
    kind: str
    summary: str
    reasoning: str
    confidence: float
    author: str
    source: str
    status: str
    created_at: datetime


class DocumentOut(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    filename: str
    content_type: str
    text_length: int
    created_at: datetime


class DocumentDetail(DocumentOut):
    text: str
    objects: list[KnowledgeObjectOut]


class ClassifyItem(BaseModel):
    kind: str
    summary: str
    reasoning: str
    confidence: float
    author: str
    source: str


class ClassifyResponse(BaseModel):
    items: list[ClassifyItem]
