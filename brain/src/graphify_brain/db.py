"""The engine's SQLite file, opened two ways.

The engine owns this database: it creates it, migrates it, and syncs the call data into
it. The brain is a guest. So there are two connections and no third:

* `read_only` for everything the brain reads — `calls`, `assistants`, `tool_calls`.
  Read-only at the SQLite level, not by convention, so a stray UPDATE in a prompt-driven
  code path fails loudly instead of quietly editing a client's call history.
* `read_write` for the three tables the brain is the author of: `patterns` and its label
  and match rows. Writable at the SQLite level too, and only those three — an authorizer
  refuses a write to any other table on this connection. That matters because this is not
  a connection that only touches its own tables: `label`, `synthesize` and `daily` all
  read transcripts out of `calls` over it, and what else it can reach from there is the
  key store, a client's call history, and the `jobs` and `spend` rows the engine binds
  into one transaction so that a job cannot close without its money landing. A second
  writer is the shape that undoes that. The engine owns those rows; the brain is a guest.

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

#: The tables the brain is the author of. Every other table in the file is the engine's.
#: This is a list of what may be written rather than of what must be protected, so a table
#: added to the engine's schema tomorrow is covered by nobody having to remember it, and a
#: brain command that grows a real need for a fourth table fails the first time it runs
#: instead of quietly writing where it should not.
AUTHORED = frozenset({"patterns", "pattern_labels", "pattern_matches"})

#: The three actions that change a row, and the only ones SQLite reports with the table's
#: own name as the first argument — which is what makes them checkable by name at all.
#: `SQLITE_ALTER_TABLE` is deliberately not here: its first argument is the *database*
#: name, so a set whose rule is "the first argument is a table" would be a rule this entry
#: does not obey. Nothing is lost by leaving both it and `SQLITE_DROP_TABLE` out, because
#: SQLite asks about the rows either way — a `DROP` deletes from the table and from
#: `sqlite_master`, and an `ALTER` updates `sqlite_master` — and `sqlite_master` is not a
#: table the brain authors either.
_TABLE_WRITES = frozenset(
    {
        sqlite3.SQLITE_INSERT,
        sqlite3.SQLITE_UPDATE,
        sqlite3.SQLITE_DELETE,
    }
)


def read_only(path: str | Path) -> sqlite3.Connection:
    """Open the engine's database for reading call data."""
    return _open(path, "ro")


def read_write(path: str | Path) -> sqlite3.Connection:
    """Open the engine's database for writing the brain's own tables."""
    conn = _open(path, "rw")
    conn.set_authorizer(_authored_tables_only)
    return conn


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


def _authored_tables_only(action: int, table: str | None, *_rest: object) -> int:
    """Refuse a write to a table the brain does not author. Allow everything else.

    Reads are deliberately untouched — three of the four commands holding this connection
    read `calls` over it, and taking that away would be a different program. SQLite runs
    this while it prepares a statement, so a refused write raises there and then rather
    than running and changing nothing.
    """
    if action in _TABLE_WRITES and table not in AUTHORED:
        return sqlite3.SQLITE_DENY
    return sqlite3.SQLITE_OK
