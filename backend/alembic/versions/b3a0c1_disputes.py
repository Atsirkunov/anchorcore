"""add disputes table + entity dispute_count (B3)

Adds:
- disputes — audit trail of who disputed an entity, when, and why
- entities.dispute_count — counter incremented on each dispute

Disputed entities are excluded from Q&A retrieval by default (see
AnswerEngine + ANCHOR_QA_EXCLUDE_DISPUTED).

Revision ID: b3a0c1
Revises: b15a0d1
Create Date: 2026-08-07 00:00:00.000000

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

# revision identifiers, used by Alembic.
revision: str = "b3a0c1"
down_revision: Union[str, None] = "b15a0d1"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column("entities", sa.Column("dispute_count", sa.Integer(), nullable=False, server_default="0"))
    op.create_table(
        "disputes",
        sa.Column("id", sa.Integer(), primary_key=True),
        sa.Column("entity_id", sa.Integer(), sa.ForeignKey("entities.id", ondelete="CASCADE"), nullable=False, index=True),
        sa.Column("reason", sa.Text(), nullable=False, server_default=""),
        sa.Column("user", sa.String(length=300), nullable=False, server_default=""),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
    )


def downgrade() -> None:
    op.drop_table("disputes")
    op.drop_column("entities", "dispute_count")
