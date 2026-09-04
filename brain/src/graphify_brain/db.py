"""The engine's SQLite file, opened two ways.

The engine owns this database: it creates it, migrates it, and syncs the call data into
it. The brain is a guest. So there are two connections and no third:

* `read_only` for everything the brain reads — `calls`, `assistants`, `tool_calls`.
  Read-only at the SQLite level, not by convention, so a stray UPDATE in a prompt-driven
  code path fails loudly instead of quietly editing a client's call history.
* `read_write` for the tables the brain is the author of — `jobs`, `patterns`, and
  their label and match rows.

Neither opens with `mode=rwc`. A wrong `--db` path must be an error the moment it is
passed; if it created an empty file instead, every query after it would fail with "no
such table" and the real mistake would be three screens back.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path

#: How long to wait for the engine to finish a write before giving up. The engine leaves
#: SQLite in its default rollback-journal mode, where a writer and a reader lock each
#: other out, so a `sync` running while a job reads is a normal collision and not an
#: error — as long as somebody waits.
BUSY_TIMEOUT_MS = 5_000


def read_only(path: str | Path) -> sqlite3.Connection:
    """Open the engine's database for reading call data."""
    return _open(path, "ro")


def read_write(path: str | Path) -> sqlite3.Connection:
    """Open the engine's database for writing `jobs` and `patterns`.

    SQLite has no per-table permission, so "only those tables" is a promise this module
    makes and cannot enforce. What it can enforce is that the connection the call data
    arrives on is not this one.
    """
    return _open(path, "rw")


def _open(path: str | Path, mode: str) -> sqlite3.Connection:
    file = Path(path)
    if not file.is_file():
        raise FileNotFoundError(
            f"no graphify database at {file}; the engine creates it — run `graphify sync` first"
        )
    # `as_uri()` percent-encodes the path, so a database under a directory with a space
    # or a `?` in its name still opens.
    conn = sqlite3.connect(f"{file.resolve().as_uri()}?mode={mode}", uri=True)
    # Columns by name. The engine's `calls` table has fifty of them and positional
    # indexing into it would be unreadable and wrong after the next migration.
    conn.row_factory = sqlite3.Row
    conn.execute(f"PRAGMA busy_timeout = {BUSY_TIMEOUT_MS}")
    return conn
