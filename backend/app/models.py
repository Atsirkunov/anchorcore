import json
import hashlib
from datetime import datetime, timezone

from sqlalchemy import DateTime, Float, ForeignKey, Integer, String, Table, Text, Column
from sqlalchemy.orm import Mapped, mapped_column, relationship

from .db import Base


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


project_sources = Table(
    "project_sources",
    Base.metadata,
    Column("project_id", ForeignKey("projects.id", ondelete="CASCADE"), primary_key=True),
    Column("source_id", ForeignKey("sources.id", ondelete="CASCADE"), primary_key=True),
)


class Project(Base):
    """A named bundle of sources (B15) — scoped search. A source can belong to
    multiple projects; one project is the user's default scope."""

    __tablename__ = "projects"

    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(300))
    is_default: Mapped[bool] = mapped_column(default=False)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)

    sources: Mapped[list["Source"]] = relationship(secondary=project_sources, back_populates="projects")


class Source(Base):
    __tablename__ = "sources"

    id: Mapped[int] = mapped_column(primary_key=True)
    connector: Mapped[str] = mapped_column(String(50))  # folder | jira | upload
    name: Mapped[str] = mapped_column(String(300))
    config: Mapped[str] = mapped_column(Text, default="{}")  # JSON, secrets referenced by key
    last_sync_cursor: Mapped[str] = mapped_column(String(500), default="")
    last_synced_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    last_error: Mapped[str | None] = mapped_column(Text, nullable=True)
    error_count: Mapped[int] = mapped_column(Integer, default=0)
    enabled: Mapped[bool] = mapped_column(default=True)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)

    items: Mapped[list["IngestedItem"]] = relationship(back_populates="source", cascade="all, delete-orphan")
    projects: Mapped[list["Project"]] = relationship(secondary=project_sources, back_populates="sources")

    def config_dict(self) -> dict:
        return json.loads(self.config or "{}")


class IngestedItem(Base):
    __tablename__ = "ingested_items"

    id: Mapped[int] = mapped_column(primary_key=True)
    source_id: Mapped[int] = mapped_column(ForeignKey("sources.id", ondelete="CASCADE"))
    external_id: Mapped[str] = mapped_column(String(500), default="")
    title: Mapped[str] = mapped_column(String(500), default="")
    text: Mapped[str] = mapped_column(Text, default="")  # raw content for chunking/backfill
    content_hash: Mapped[str] = mapped_column(String(64), index=True)
    author: Mapped[str] = mapped_column(String(300), default="")
    updated_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    stale: Mapped[bool] = mapped_column(default=False)
    doc_type: Mapped[str] = mapped_column(String(50), default="")  # standards|meeting|decision_log|prd|runbook|general
    window_hashes: Mapped[str] = mapped_column(Text, default="{}")  # JSON {index: sha256}
    distill_hashes: Mapped[str] = mapped_column(Text, default="{}")  # JSON {index: sha256} — B18 distillation
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)

    source: Mapped[Source] = relationship(back_populates="items")
    entities: Mapped[list["Entity"]] = relationship(back_populates="item", cascade="all, delete-orphan")
    chunks: Mapped[list["Chunk"]] = relationship(back_populates="item", cascade="all, delete-orphan")


class Entity(Base):
    __tablename__ = "entities"

    id: Mapped[int] = mapped_column(primary_key=True)
    item_id: Mapped[int] = mapped_column(ForeignKey("ingested_items.id", ondelete="CASCADE"))
    kind: Mapped[str] = mapped_column(String(50), index=True)  # decision|document|action|note
    summary: Mapped[str] = mapped_column(Text, default="")
    reasoning: Mapped[str] = mapped_column(Text, default="")
    confidence: Mapped[float] = mapped_column(Float, default=0.0)
    author: Mapped[str] = mapped_column(String(300), default="")
    source_ref: Mapped[str] = mapped_column(String(500), default="")  # source + section
    window_text: Mapped[str] = mapped_column(Text, default="")  # classifier input window
    window_index: Mapped[int | None] = mapped_column(Integer, nullable=True)  # window ordinal
    status: Mapped[str] = mapped_column(String(50), default="unverified", index=True)
    owner: Mapped[str] = mapped_column(String(300), default="")
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, onupdate=utcnow)

    item: Mapped[IngestedItem] = relationship(back_populates="entities")
    chunks: Mapped[list["Chunk"]] = relationship(back_populates="entity", cascade="all, delete-orphan")
    outgoing: Mapped[list["Relationship"]] = relationship(
        back_populates="from_entity", foreign_keys="Relationship.from_entity_id", passive_deletes=True
    )
    incoming: Mapped[list["Relationship"]] = relationship(
        back_populates="to_entity", foreign_keys="Relationship.to_entity_id", passive_deletes=True
    )
    merge_proposals: Mapped[list["MergeAction"]] = relationship(
        back_populates="entity_a", foreign_keys="MergeAction.entity_a_id", passive_deletes=True
    )

    def __repr__(self) -> str:
        return f"<Entity {self.id} {self.kind}: {self.summary[:40]}>"


class Relationship(Base):
    __tablename__ = "relationships"

    id: Mapped[int] = mapped_column(primary_key=True)
    from_entity_id: Mapped[int] = mapped_column(ForeignKey("entities.id", ondelete="CASCADE"))
    to_entity_id: Mapped[int] = mapped_column(ForeignKey("entities.id", ondelete="CASCADE"))
    kind: Mapped[str] = mapped_column(String(100), default="related")  # supersedes|depends_on|owns|blocks
    source_ref: Mapped[str] = mapped_column(String(500), default="")
    confidence: Mapped[float] = mapped_column(Float, default=0.0)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)

    from_entity: Mapped[Entity] = relationship(back_populates="outgoing", foreign_keys=[from_entity_id])
    to_entity: Mapped[Entity] = relationship(back_populates="incoming", foreign_keys=[to_entity_id])


class Chunk(Base):
    __tablename__ = "chunks"

    id: Mapped[int] = mapped_column(primary_key=True)
    item_id: Mapped[int | None] = mapped_column(
        ForeignKey("ingested_items.id", ondelete="CASCADE"), nullable=True
    )  # set for full-document chunks
    entity_id: Mapped[int | None] = mapped_column(
        ForeignKey("entities.id", ondelete="CASCADE"), nullable=True
    )  # set for entity-summary chunks
    kind: Mapped[str] = mapped_column(String(20), default="document", server_default="document")  # document|entity|distilled
    source_ref: Mapped[str] = mapped_column(String(500), default="")
    content: Mapped[str] = mapped_column(Text, default="")
    embedding: Mapped[bytes | None] = mapped_column(nullable=True)  # float32 blob
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)

    entity: Mapped[Entity | None] = relationship(back_populates="chunks")
    item: Mapped[IngestedItem | None] = relationship(back_populates="chunks")


class Job(Base):
    __tablename__ = "jobs"

    id: Mapped[int] = mapped_column(primary_key=True)
    source_id: Mapped[int] = mapped_column(ForeignKey("sources.id", ondelete="CASCADE"), index=True)
    kind: Mapped[str] = mapped_column(String(50))  # sync | reclassify
    status: Mapped[str] = mapped_column(String(20), default="running", index=True)  # running|done|failed
    total: Mapped[int] = mapped_column(Integer, default=0)
    processed: Mapped[int] = mapped_column(Integer, default=0)
    result: Mapped[str] = mapped_column(Text, default="{}")  # JSON summary of what happened
    error: Mapped[str | None] = mapped_column(Text, nullable=True)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)
    started_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    finished_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)

    source: Mapped[Source] = relationship()

    def result_dict(self) -> dict:
        return json.loads(self.result or "{}")


class SystemEvent(Base):
    __tablename__ = "system_events"

    id: Mapped[int] = mapped_column(primary_key=True)
    component: Mapped[str] = mapped_column(String(50), index=True)  # pipeline|connector|scheduler|embedder|qa|system
    level: Mapped[str] = mapped_column(String(20), default="error", index=True)  # error|warning|info
    source_id: Mapped[int | None] = mapped_column(
        ForeignKey("sources.id", ondelete="SET NULL"), nullable=True
    )
    message: Mapped[str] = mapped_column(Text, default="")
    detail: Mapped[str] = mapped_column(Text, default="")
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)

    source: Mapped[Source | None] = relationship()


class AppSetting(Base):
    """Runtime-mutable app settings (B4): DB values override .env defaults.

    Secrets (API keys) are NOT stored here — they live in the SecretStore
    under keys prefixed `app:`. This table holds non-secret overrides.
    """

    __tablename__ = "app_settings"

    key: Mapped[str] = mapped_column(String(100), primary_key=True)
    value: Mapped[str] = mapped_column(Text, default="")
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), default=utcnow, onupdate=utcnow
    )


class MergeAction(Base):
    __tablename__ = "merge_actions"

    id: Mapped[int] = mapped_column(primary_key=True)
    entity_a_id: Mapped[int] = mapped_column(ForeignKey("entities.id", ondelete="CASCADE"))
    entity_b_id: Mapped[int] = mapped_column(ForeignKey("entities.id", ondelete="CASCADE"), nullable=True)
    status: Mapped[str] = mapped_column(String(50), default="proposed")  # proposed|merged|dismissed
    reason: Mapped[str] = mapped_column(String(500), default="")
    user: Mapped[str] = mapped_column(String(300), default="")
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)

    entity_a: Mapped[Entity] = relationship(back_populates="merge_proposals", foreign_keys=[entity_a_id])
    entity_b: Mapped[Entity] = relationship(foreign_keys=[entity_b_id])


def content_hash(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()
