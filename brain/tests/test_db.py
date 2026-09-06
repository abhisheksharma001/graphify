"""Which connection can write, and which cannot.

The tables here are made by hand rather than from the engine's migration: what is under
test is the connection mode, and reaching across into `engine/migrations/` would tie the
Python tests to the Rust crate's layout for no extra coverage.
"""

import re
import sqlite3
from pathlib import Path

import pytest

from graphify_brain import db


@pytest.fixture
def store(tmp_path):
    """An engine-shaped database: a row to read, and one table on each side of the line.

    `calls`, `jobs`, `spend` and `secrets` belong to the engine; `patterns` is the
    brain's. Enough
    of the schema to tell the two apart, and no more — see the module docstring.
    """
    path = tmp_path / "graphify.db"
    conn = sqlite3.connect(path)
    conn.execute("CREATE TABLE calls (id TEXT PRIMARY KEY, summary TEXT)")
    conn.execute("CREATE TABLE jobs (id INTEGER PRIMARY KEY, kind TEXT, cost_usd REAL)")
    conn.execute("CREATE TABLE spend (day TEXT, org_id INTEGER, usd REAL)")
    conn.execute("CREATE TABLE secrets (org_id INTEGER, name TEXT, sealed BLOB)")
    conn.execute("CREATE TABLE patterns (id INTEGER PRIMARY KEY, name TEXT)")
    conn.execute("INSERT INTO calls VALUES ('c1', 'asked for a human')")
    conn.commit()
    conn.close()
    return path


def test_read_only_reads_columns_by_name(store):
    row = db.read_only(store).execute("SELECT * FROM calls").fetchone()

    assert row["id"] == "c1"
    assert row["summary"] == "asked for a human"


def test_read_only_cannot_write(store):
    conn = db.read_only(store)

    with pytest.raises(sqlite3.OperationalError, match="readonly"):
        conn.execute("UPDATE calls SET summary = 'edited'")


def test_read_write_writes_the_tables_the_brain_authors(store):
    conn = db.read_write(store)
    conn.execute("INSERT INTO patterns (name) VALUES ('asked for a human')")
    conn.commit()

    assert db.read_only(store).execute("SELECT count(*) FROM patterns").fetchone()[0] == 1


def test_read_write_refuses_the_engines_tables(store):
    """The connection that reads transcripts can also reach the key store, the call
    history, and the `jobs` and `spend` rows the engine moves together. It may not write
    any of them. A refusal is raised while the statement is prepared, so a mistake is an
    error and not a write that silently did nothing."""
    conn = db.read_write(store)

    for sql in (
        "UPDATE calls SET summary = 'edited'",
        "DELETE FROM calls",
        "INSERT INTO jobs (kind, cost_usd) VALUES ('label', 0.22)",
        "INSERT INTO spend VALUES ('2026-01-01', 1, 99.0)",
        "INSERT INTO secrets VALUES (1, 'vapi', x'00')",
        # Refusing to empty a table while allowing it to be dropped would be a control
        # that reads well and holds nothing. Neither of these is refused by name: SQLite
        # asks about the rows underneath them, and `sqlite_master` is not ours either.
        "DROP TABLE calls",
        "ALTER TABLE calls ADD COLUMN leaked TEXT",
        "ALTER TABLE calls RENAME TO gone",
    ):
        with pytest.raises(sqlite3.DatabaseError, match="not authorized"):
            conn.execute(sql)

    assert conn.execute("SELECT count(*) FROM calls").fetchone()[0] == 1


def test_read_write_still_reads_the_call_data(store):
    """`label`, `synthesize` and `daily` all read transcripts on this connection. The
    authorizer is about writing; taking the reads away would be a different program."""
    row = db.read_write(store).execute("SELECT summary FROM calls").fetchone()

    assert row["summary"] == "asked for a human"


def test_the_authored_set_is_the_set_the_brain_actually_writes():
    """The guard the two tests above rest on. `AUTHORED` is a decision, not something
    that can be derived — but what the brain writes today can be, so the two are held
    against each other. An `INSERT INTO jobs` added to a command fails here rather than
    failing in front of a client; a name added to `AUTHORED` with no caller fails here
    too, because a permission nothing uses is one nobody chose.
    """
    package = Path(db.__file__).parent
    writes = re.compile(r"INSERT(?: OR \w+)? INTO (\w+)|UPDATE (\w+) SET|DELETE FROM (\w+)")

    written = {
        next(name for name in match.groups() if name)
        for source in package.glob("*.py")
        for match in writes.finditer(source.read_text())
    }

    assert written, "the harvest found no SQL anywhere in the package, which cannot be right"
    assert written == set(db.AUTHORED)


def test_a_missing_database_is_an_error_and_stays_missing(tmp_path):
    """Not `mode=rwc`: a typo in `--db` must not become an empty database that then
    fails on every query with "no such table"."""
    missing = tmp_path / "nowhere.db"

    for open_it in (db.read_only, db.read_write):
        with pytest.raises(FileNotFoundError, match="no graphify database"):
            open_it(missing)

    assert not missing.exists()


def test_a_path_with_a_space_opens(tmp_path):
    """`as_uri()` percent-encodes, so this is a real path and not a malformed URI."""
    directory = tmp_path / "my calls"
    directory.mkdir()
    path = directory / "graphify.db"
    conn = sqlite3.connect(path)
    conn.execute("CREATE TABLE calls (id TEXT)")
    conn.commit()
    conn.close()

    assert db.read_only(path).execute("SELECT count(*) FROM calls").fetchone()[0] == 0
