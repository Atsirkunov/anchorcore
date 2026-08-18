"""add vec_chunks (sqlite-vec vec0 index for vector retrieval — B33)

Mirrors the chunks_fts FTS5 pattern: a virtual table kept in sync by AFTER
triggers on `chunks`, backfilled from existing embeddings at migration time.

vec0 requires a fixed dimension at table creation. The default embed model
(nomic-embed-text) produces 768-dim vectors; the trigger WHEN-guards on that
length so a differently-sized embed model never breaks chunk writes — the
index simply stays empty and vector retrieval falls back to the pure-Python
cosine scan.

Revision ID: b33a0c1
Revises: b3a0c1
Create Date: 2026-08-10 00:00:00.000000

"""
import logging
from typing import Sequence, Union

from alembic import op

logger = logging.getLogger(__name__)

# revision identifiers, used by Alembic.
revision: str = "b33a0c1"
down_revision: Union[str, None] = "b3a0c1"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None

_DIM = 768  # nomic-embed-text default (matches config.embed_dim)
_DIM_BYTES = _DIM * 4  # float32

_VEC = f"""
CREATE VIRTUAL TABLE vec_chunks USING vec0(
    embedding float[{_DIM}] distance_metric=cosine
)
"""

_TRIGGER_INSERT = f"""
CREATE TRIGGER vec_chunks_ai AFTER INSERT ON chunks
WHEN new.embedding IS NOT NULL AND length(new.embedding) = {_DIM_BYTES}
BEGIN
    INSERT INTO vec_chunks(rowid, embedding) VALUES (new.id, new.embedding);
END
"""

_TRIGGER_DELETE = """
CREATE TRIGGER vec_chunks_ad AFTER DELETE ON chunks BEGIN
    DELETE FROM vec_chunks WHERE rowid = old.id;
END
"""

_TRIGGER_UPDATE = f"""
CREATE TRIGGER vec_chunks_au AFTER UPDATE OF embedding ON chunks
WHEN new.embedding IS NOT NULL AND length(new.embedding) = {_DIM_BYTES}
BEGIN
    DELETE FROM vec_chunks WHERE rowid = old.id;
    INSERT INTO vec_chunks(rowid, embedding) VALUES (new.id, new.embedding);
END
"""

_BACKFILL = f"""
INSERT INTO vec_chunks(rowid, embedding)
SELECT id, embedding FROM chunks
WHERE embedding IS NOT NULL AND length(embedding) = {_DIM_BYTES}
"""


def upgrade() -> None:
    if op.get_context().dialect.name == "postgresql":
        return
    try:
        op.execute(_VEC)
        op.execute(_TRIGGER_INSERT)
        op.execute(_TRIGGER_DELETE)
        op.execute(_TRIGGER_UPDATE)
        op.execute(_BACKFILL)
    except Exception as exc:  # noqa: BLE001
        # sqlite-vec unavailable (e.g. Python compiled without loadable
        # extensions): skip the index — vector retrieval falls back to the
        # pure-Python cosine scan, so the app still works.
        logger.warning(
            "vec0 index unavailable (%s); vector retrieval uses Python scan", exc
        )


def downgrade() -> None:
    if op.get_context().dialect.name == "postgresql":
        return
    op.execute("DROP TRIGGER IF EXISTS vec_chunks_au")
    op.execute("DROP TRIGGER IF EXISTS vec_chunks_ad")
    op.execute("DROP TRIGGER IF EXISTS vec_chunks_ai")
    op.execute("DROP TABLE IF EXISTS vec_chunks")
