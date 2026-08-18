"""add chunks.is_pii + chunks.pii_categories (B30 PII review surface)

Chunks auto-flagged by the PII scan carry `is_pii` (strong/confirmed match)
and a JSON list of the matched category ids so the review page can show what
matched and let a user confirm/override the classification.

Revision ID: b30a0c1
Revises: b39a0c1
Create Date: 2026-08-12 00:00:00.000000

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

# revision identifiers, used by Alembic.
revision: str = "b30a0c1"
down_revision: Union[str, None] = "b39a0c1"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column(
        "chunks",
        sa.Column("is_pii", sa.Boolean(), nullable=False, server_default=sa.false()),
    )
    op.add_column(
        "chunks",
        sa.Column("pii_categories", sa.Text(), nullable=False, server_default="[]"),
    )


def downgrade() -> None:
    op.drop_column("chunks", "pii_categories")
    op.drop_column("chunks", "is_pii")
