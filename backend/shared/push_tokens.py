"""SQLite-backed push token store.

Single file at $STATE_DIR/push_tokens.sqlite. Tokens are unique per user +
platform; re-registering the same token is a no-op (UPSERT). Lookups are
either by user_id (for fan-out from a watch) or by token (for cleanup
when APNS reports BadDeviceToken).

Schema is small enough we keep it inline — migrations come if the schema
ever grows beyond two columns.
"""

from __future__ import annotations

import os
import sqlite3
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path


STATE_DIR = Path(os.environ.get("STATE_DIR", "/data/state"))
DB_PATH = STATE_DIR / "push_tokens.sqlite"


@dataclass
class PushToken:
    user_id: str
    token: str
    platform: str  # "ios" | "android"
    created_at: float


def _ensure_schema(conn: sqlite3.Connection) -> None:
    conn.executescript(
        """
        CREATE TABLE IF NOT EXISTS push_tokens (
            user_id    TEXT NOT NULL,
            token      TEXT NOT NULL PRIMARY KEY,
            platform   TEXT NOT NULL,
            created_at REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_user ON push_tokens(user_id);
        """
    )


@contextmanager
def _connect():
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(DB_PATH))
    try:
        _ensure_schema(conn)
        yield conn
        conn.commit()
    finally:
        conn.close()


def upsert(user_id: str, token: str, platform: str) -> None:
    import time as _t
    with _connect() as conn:
        conn.execute(
            "INSERT INTO push_tokens(user_id, token, platform, created_at) "
            "VALUES(?, ?, ?, ?) "
            "ON CONFLICT(token) DO UPDATE SET user_id=excluded.user_id, platform=excluded.platform",
            (user_id, token, platform, _t.time()),
        )


def delete_by_token(token: str) -> int:
    with _connect() as conn:
        cur = conn.execute("DELETE FROM push_tokens WHERE token = ?", (token,))
        return cur.rowcount


def delete_for_user(user_id: str, token: str) -> int:
    with _connect() as conn:
        cur = conn.execute(
            "DELETE FROM push_tokens WHERE user_id = ? AND token = ?",
            (user_id, token),
        )
        return cur.rowcount


def list_for_user(user_id: str) -> list[PushToken]:
    with _connect() as conn:
        rows = conn.execute(
            "SELECT user_id, token, platform, created_at FROM push_tokens WHERE user_id = ?",
            (user_id,),
        ).fetchall()
    return [PushToken(*r) for r in rows]
