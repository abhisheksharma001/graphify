"""Which connection can write, and which cannot.

The tables here are made by hand rather than from the engine's migration: what is under
test is the connection mode, and reaching across into `engine/migrations/` would tie the
Python tests to the Rust crate's layout for no extra coverage.
"""

import sqlite3

import pytest

from graphify_brain import db


@pytest.fixture
def store(tmp_path):
    """An engine-shaped database: one row to read, one table to write."""
    path = tmp_path / "graphify.db"
    conn = sqlite3.connect(path)
    conn.execute("CREATE TABLE calls (id TEXT PRIMARY KEY, summary TEXT)")
    conn.execute("CREATE TABLE jobs (id INTEGER PRIMARY KEY, kind TEXT, cost_usd REAL)")
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


def test_read_write_writes_jobs(store):
    conn = db.read_write(store)
    conn.execute("INSERT INTO jobs (kind, cost_usd) VALUES ('label', 0.22)")
    conn.commit()

    assert db.read_only(store).execute("SELECT count(*) FROM jobs").fetchone()[0] == 1


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
