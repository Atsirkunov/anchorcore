"""add chunks_fts (FTS5 keyword index for hybrid retrieval)

Revision ID: b12f7c0
Revises: 7c4a2b9e3d81
Create Date: 2026-08-05 09:00:00.000000

"""
from typing import Sequence, Union

from alembic import op

# revision identifiers, used by Alembic.
revision: str = "b12f7c0"
down_revision: Union[str, None] = "7c4a2b9e3d81"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None

_FTS = """
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    content,
    content='chunks',
    content_rowid='id',
    tokenize='porter unicode61'
)
"""

_TRIGGER_INSERT = """
CREATE TRIGGER chunks_fts_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts(rowid, content) VALUES (new.id, new.content);
END
"""

_TRIGGER_DELETE = """
CREATE TRIGGER chunks_fts_ad AFTER DELETE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, content) VALUES ('delete', old.id, old.content);
END
"""

_TRIGGER_UPDATE = """
CREATE TRIGGER chunks_fts_au AFTER UPDATE OF content ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, content) VALUES ('delete', old.id, old.content);
    INSERT INTO chunks_fts(rowid, content) VALUES (new.id, new.content);
END
"""


def _is_postgres() -> bool:
    return op.get_context().dialect.name == "postgresql"


def upgrade() -> None:
    if _is_postgres():
        return
    op.execute(_FTS)
    op.execute(_TRIGGER_INSERT)
    op.execute(_TRIGGER_DELETE)
    op.execute(_TRIGGER_UPDATE)
    op.execute("INSERT INTO chunks_fts(rowid, content) SELECT id, content FROM chunks")


def downgrade() -> None:
    if _is_postgres():
        return
    op.execute("DROP TRIGGER IF EXISTS chunks_fts_au")
    op.execute("DROP TRIGGER IF EXISTS chunks_fts_ad")
    op.execute("DROP TRIGGER IF EXISTS chunks_fts_ai")
    op.execute("DROP TABLE IF EXISTS chunks_fts")
