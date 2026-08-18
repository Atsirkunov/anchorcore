"""add users table (hosted auth skeleton B40)

Local stays no-auth; when ANCHOR_AUTH_SECRET is set, /auth signup/login
issues Bearer tokens. Minimal table, no FKs.

Revision ID: b40a0c1
Revises: b30a0c1
Create Date: 2026-08-17 00:00:00.000000
"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "b40a0c1"
down_revision: Union[str, None] = "b30a0c1"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.create_table(
        "users",
        sa.Column("id", sa.Integer(), primary_key=True),
        sa.Column("email", sa.String(length=320), nullable=False),
        sa.Column("password_hash", sa.String(length=300), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
    )
    op.create_index("ix_users_email", "users", ["email"], unique=True)


def downgrade() -> None:
    op.drop_index("ix_users_email", table_name="users")
    op.drop_table("users")
