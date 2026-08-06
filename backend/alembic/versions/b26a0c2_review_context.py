"""entity review context + per-window classification (B26)

Adds:
- entities.window_text / entities.window_index — the classifier input window
  an entity came from, so the Review tab can show what the model actually saw
- ingested_items.doc_type — detected document type (standards|meeting|...)
- ingested_items.window_hashes — per-window content hashes so reclassify can
  skip unchanged windows (cheaper cloud classification)

Revision ID: b26a0c2
Revises: b4a00c1
Create Date: 2026-08-06 20:00:00.000000

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

# revision identifiers, used by Alembic.
revision: str = "b26a0c2"
down_revision: Union[str, None] = "b4a00c1"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column("entities", sa.Column("window_text", sa.Text(), nullable=False, server_default=""))
    op.add_column("entities", sa.Column("window_index", sa.Integer(), nullable=True))
    op.add_column("ingested_items", sa.Column("doc_type", sa.String(length=50), nullable=False, server_default=""))
    op.add_column("ingested_items", sa.Column("window_hashes", sa.Text(), nullable=False, server_default="{}"))


def downgrade() -> None:
    op.drop_column("ingested_items", "window_hashes")
    op.drop_column("ingested_items", "doc_type")
    op.drop_column("entities", "window_index")
    op.drop_column("entities", "window_text")
