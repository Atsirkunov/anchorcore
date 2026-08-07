"""add chunks.kind + distilled unit support (B18)

Adds:
- chunks.kind — discriminator: document (full-doc) | entity (entity-summary)
  | distilled (normalized Q&A unit from the distillation pass)
- ingested_items.distill_hashes — per-window content hashes for the distillation
  pass, so reclassify can skip unchanged windows (mirrors window_hashes)

Backfill: existing chunks keep kind='document' (default). Entity-summary chunks
(those with entity_id set) are marked kind='entity'.

Revision ID: b18d0c1
Revises: b26a0c2
Create Date: 2026-08-07 00:00:00.000000

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

# revision identifiers, used by Alembic.
revision: str = "b18d0c1"
down_revision: Union[str, None] = "b26a0c2"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column(
        "chunks",
        sa.Column("kind", sa.String(length=20), nullable=False, server_default="document"),
    )
    op.add_column(
        "ingested_items",
        sa.Column("distill_hashes", sa.Text(), nullable=False, server_default="{}"),
    )
    # mark existing entity-summary chunks (entity_id set) as kind='entity'
    op.execute("UPDATE chunks SET kind='entity' WHERE entity_id IS NOT NULL")


def downgrade() -> None:
    op.drop_column("ingested_items", "distill_hashes")
    op.drop_column("chunks", "kind")
