"""add sources.label — data classification for PII gating (B39 thin gate)

Labels: `internal` (default) | `public` | `sensitive` | `pii`. The B39
provider-trust gate refuses to send `sensitive`/`pii` content to cloud
classify/embed providers (fallback to rule-based + system_event) until the
user confirms the provider is trusted.

Revision ID: b39a0c1
Revises: b33a0c1
Create Date: 2026-08-11 00:00:00.000000

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

# revision identifiers, used by Alembic.
revision: str = "b39a0c1"
down_revision: Union[str, None] = "b33a0c1"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column(
        "sources",
        sa.Column("label", sa.String(length=20), nullable=False, server_default="internal"),
    )


def downgrade() -> None:
    op.drop_column("sources", "label")
