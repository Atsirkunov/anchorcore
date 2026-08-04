"""add system_events table

Revision ID: 7c4a2b9e3d81
Revises: 9d3f1b2c4a51
Create Date: 2026-08-04 12:10:00.000000

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision: str = "7c4a2b9e3d81"
down_revision: Union[str, None] = "9d3f1b2c4a51"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.create_table(
        "system_events",
        sa.Column("id", sa.Integer(), nullable=False),
        sa.Column("component", sa.String(length=50), nullable=False),
        sa.Column("level", sa.String(length=20), nullable=False),
        sa.Column("source_id", sa.Integer(), nullable=True),
        sa.Column("message", sa.Text(), nullable=False),
        sa.Column("detail", sa.Text(), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.ForeignKeyConstraint(["source_id"], ["sources.id"], ondelete="SET NULL"),
        sa.PrimaryKeyConstraint("id"),
    )
    op.create_index(op.f("ix_system_events_component"), "system_events", ["component"], unique=False)
    op.create_index(op.f("ix_system_events_level"), "system_events", ["level"], unique=False)


def downgrade() -> None:
    op.drop_index(op.f("ix_system_events_level"), table_name="system_events")
    op.drop_index(op.f("ix_system_events_component"), table_name="system_events")
    op.drop_table("system_events")
