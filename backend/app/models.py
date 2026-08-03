import json
import hashlib
from datetime import datetime, timezone

from sqlalchemy import DateTime, Float, ForeignKey, Integer, String, Text
from sqlalchemy.orm import Mapped, mapped_column, relationship

from .db import Base


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


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

    def config_dict(self) -> dict:
        return json.loads(self.config or "{}")


class IngestedItem(Base):
    __tablename__ = "ingested_items"

    id: Mapped[int] = mapped_column(primary_key=True)
    source_id: Mapped[int] = mapped_column(ForeignKey("sources.id", ondelete="CASCADE"))
    external_id: Mapped[str] = mapped_column(String(500), default="")
    title: Mapped[str] = mapped_column(String(500), default="")
    content_hash: Mapped[str] = mapped_column(String(64), index=True)
    author: Mapped[str] = mapped_column(String(300), default="")
    updated_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    stale: Mapped[bool] = mapped_column(default=False)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)

    source: Mapped[Source] = relationship(back_populates="items")
    entities: Mapped[list["Entity"]] = relationship(back_populates="item")


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
    status: Mapped[str] = mapped_column(String(50), default="unverified", index=True)
    owner: Mapped[str] = mapped_column(String(300), default="")
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, onupdate=utcnow)

    item: Mapped[IngestedItem] = relationship(back_populates="entities")
    chunks: Mapped[list["Chunk"]] = relationship(back_populates="entity", cascade="all, delete-orphan")
    outgoing: Mapped[list["Relationship"]] = relationship(
        back_populates="from_entity", foreign_keys="Relationship.from_entity_id"
    )
    incoming: Mapped[list["Relationship"]] = relationship(
        back_populates="to_entity", foreign_keys="Relationship.to_entity_id"
    )
    merge_proposals: Mapped[list["MergeAction"]] = relationship(back_populates="entity_a", foreign_keys="MergeAction.entity_a_id")

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
    entity_id: Mapped[int] = mapped_column(ForeignKey("entities.id", ondelete="CASCADE"))
    source_ref: Mapped[str] = mapped_column(String(500), default="")
    content: Mapped[str] = mapped_column(Text, default="")
    embedding: Mapped[bytes | None] = mapped_column(nullable=True)  # float32 blob
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)

    entity: Mapped[Entity] = relationship(back_populates="chunks")


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
