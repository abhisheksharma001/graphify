"""The daily run, which is the first thing in graphify that spends money with nobody
watching. So every test here is about a bound: which calls are read, which are not, and
what stops the reading.

`label.call_batch` is replaced for every test, by the same `Batches` stand-in `test_label`
uses — nothing here can reach a provider, and "two batches and then the cap" is assertable
because the fake charges what the test told it to.
"""

import json
import sqlite3
import threading
from pathlib import Path

import pytest
from typer.testing import CliRunner

from baml_client import types
from graphify_brain import daily as dailies
from graphify_brain import label as labelling
from graphify_brain.cli import app

MIGRATION = Path(__file__).resolve().parents[2] / "engine" / "migrations" / "0001_init.sql"

#: Long enough that a batch costs a measurable fraction of a cent, so the cap tests are
#: about arithmetic rather than floating-point dust.
LINES = "\n".join(
    [
        "user: hi, is anyone actually there",
        "bot: I can help with that. What are you calling about?",
        "user: I want to talk to a person please",
        "bot: Let me put you through.",
    ]
)


class Never:
    """Reaching for anything on this is reaching for a model. That is the failure."""

    def __getattr__(self, name):
        raise AssertionError(f"a test called {name}; no test may call a model")


@pytest.fixture(autouse=True)
def no_model(monkeypatch):
    monkeypatch.setattr(labelling, "client", Never)


class Batches:
    """Stands in for `label.call_batch`: remembers every batch and charges a fixed price."""

    def __init__(self, usd, answer):
        self.sent = []
        self.usd = usd
        self.answer = answer
        self.lock = threading.Lock()

    def __call__(self, job, batch):
        with self.lock:
            self.sent.append(list(batch))
        return self.answer(batch), self.usd

    @property
    def read(self):
        """Every call id that reached a model, in the order the batches were built."""
        return [c.id for batch in self.sent for c in batch]


def all_match(batch):
    return [types.Label(n=i + 1, match=True, evidence="user: I want to talk to a person please") for i in range(len(batch))]


def none_match(batch):
    return [types.Label(n=i + 1, match=False, evidence="nobody asked for a person") for i in range(len(batch))]


@pytest.fixture
def batches(monkeypatch):
    def install(usd=0.0, answer=all_match):
        fake = Batches(usd, answer)
        monkeypatch.setattr(labelling, "call_batch", fake)
        return fake

    return install


@pytest.fixture
def store(tmp_path):
    """An engine-shaped database. The columns are the ones this module reads and writes,
    and `test_the_columns_this_touches_are_the_ones_the_engine_makes` checks the copy
    against the engine's own migration."""
    path = tmp_path / "graphify.db"
    conn = sqlite3.connect(path)
    conn.executescript(
        """
        CREATE TABLE calls (
          id TEXT PRIMARY KEY, org_id INTEGER, assistant_id TEXT, created_at TEXT,
          transcript TEXT, duration_s REAL, ended_reason TEXT, ended_group TEXT,
          transferred INTEGER, tool_calls INTEGER, tool_failures INTEGER
        );
        CREATE TABLE tool_calls (call_id TEXT, name TEXT, seconds_from_start REAL, failed INTEGER);
        CREATE TABLE patterns (
          id INTEGER PRIMARY KEY, org_id INTEGER, name TEXT, criterion TEXT,
          assistant_ids JSON, plan JSON, rule JSON, chart JSON, model TEXT,
          mode TEXT DEFAULT 'free', daily_cap_usd REAL DEFAULT 1.0, sample_size INTEGER,
          agreement REAL, created_at TEXT
        );
        CREATE TABLE pattern_labels (
          pattern_id INTEGER, call_id TEXT, llm_match INTEGER, rule_match INTEGER, evidence TEXT
        );
        CREATE TABLE pattern_matches (pattern_id INTEGER, call_id TEXT, source TEXT);
        """
    )
    conn.commit()
    conn.close()
    return path


def seed(store, n, org=1, first=1, assistant_id="a1", **over):
    """`n` calls, oldest first, so `c1` is the oldest and `cn` the newest."""
    row = {
        "transcript": LINES,
        "duration_s": 92.0,
        "ended_reason": "customer-ended-call",
        "ended_group": "customer",
        "transferred": 0,
        "tool_calls": 0,
        "tool_failures": 0,
    } | over
    ids = [f"c{first + i}" for i in range(n)]
    conn = sqlite3.connect(store)
    conn.executemany(
        "INSERT INTO calls (id, org_id, assistant_id, created_at, transcript, duration_s, "
        "ended_reason, ended_group, transferred, tool_calls, tool_failures) "
        "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        [
            (i, org, assistant_id, f"2026-09-01T09:{first + k:02d}:00.000Z", *row.values())
            for k, i in enumerate(ids)
        ],
    )
    conn.commit()
    conn.close()
    return ids


def a_plan():
    return types.Plan(
        rows=[types.PlanRow(if_="the caller asks for a person", then="counts as a match")],
        questions=[],
        confidence=1.0,
        expressible=True,
        reason="Nothing in the sentence reads two ways.",
    )


def pattern(store, mode="full", **over):
    """One model-backed pattern, complete unless a test breaks it on purpose."""
    row = {
        "org_id": 1,
        "name": "Handoff requests",
        "criterion": "calls where the caller asked for a person",
        "assistant_ids": None,
        "plan": a_plan().model_dump_json(),
        "rule": json.dumps({"any_phrases": ["a person"]}),
        "model": "sonnet",
        "mode": mode,
        "daily_cap_usd": 1.0,
    } | over
    conn = sqlite3.connect(store)
    cursor = conn.execute(
        "INSERT INTO patterns (org_id, name, criterion, assistant_ids, plan, rule, model, "
        "mode, daily_cap_usd) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        tuple(row.values()),
    )
    conn.commit()
    got = cursor.lastrowid
    conn.close()
    return got


def rule_matched(store, pattern_id, ids):
    """What `graphify apply` would have left behind for a hybrid pattern."""
    conn = sqlite3.connect(store)
    conn.executemany(
        "INSERT INTO pattern_matches (pattern_id, call_id, source) VALUES (?, ?, 'rule')",
        [(pattern_id, i) for i in ids],
    )
    conn.commit()
    conn.close()


def already_read(store, pattern_id, ids, match=True):
    """Calls a previous run paid to read."""
    conn = sqlite3.connect(store)
    conn.executemany(
        "INSERT INTO pattern_labels (pattern_id, call_id, llm_match, rule_match, evidence) "
        "VALUES (?, ?, ?, NULL, 'seen')",
        [(pattern_id, i, int(match)) for i in ids],
    )
    conn.commit()
    conn.close()


def rows(store, sql, *args):
    conn = sqlite3.connect(store)
    conn.row_factory = sqlite3.Row
    got = [dict(r) for r in conn.execute(sql, args)]
    conn.close()
    return got


def run(store, org=1, max_usd=100.0):
    """The command as the engine runs it: one line of JSON on stdin."""
    body = json.dumps({"org": org, "max_usd": max_usd})
    return CliRunner().invoke(app, ["daily", "--db", str(store)], input=body + "\n")


def answer(result):
    """The run's own last line — the one the engine books the spend from."""
    return json.loads(result.stdout.strip().splitlines()[-1])


# --- the acceptance -------------------------------------------------------------------


def test_a_penny_cap_runs_at_most_one_batch_and_says_cap_reached(store, batches):
    """The register's acceptance. A batch of twenty of these transcripts is estimated well
    above a cent, so the cap has to stop the run before the first one is sent — not notice
    afterwards that it was passed."""
    seed(store, 45)
    pattern(store, daily_cap_usd=0.01)
    fake = batches(usd=0.02)

    result = run(store)

    assert result.exit_code == 0
    assert len(fake.sent) <= 1
    assert fake.sent == []
    assert "cap reached" in result.output
    assert answer(result)["usd"] == 0.0


def test_the_run_stops_when_the_days_budget_is_gone(store, batches):
    """The other cap, and the one that binds the whole run rather than one pattern. The
    first pattern eats the budget; the second is never read, and the last line says so."""
    ids = seed(store, 20)
    first = pattern(store, name="one", daily_cap_usd=10.0)
    pattern(store, name="two", daily_cap_usd=10.0)
    fake = batches(usd=0.40)

    result = run(store, max_usd=0.40)

    assert answer(result)["stopped"] == "cap"
    assert fake.read == list(reversed(ids))
    assert [p["pattern"] for p in answer(result)["patterns"]] == [first]
    assert answer(result)["usd"] == pytest.approx(0.40)


# --- which calls are read -------------------------------------------------------------


def test_hybrid_reads_only_the_calls_its_rule_matched(store, batches):
    """D-8: in hybrid the rule is a prefilter. Reading a call it did not select would be
    paying full-mode prices for a mode chosen to avoid them."""
    seed(store, 6)
    p = pattern(store, mode="hybrid")
    rule_matched(store, p, ["c2", "c4"])
    fake = batches()

    run(store)

    assert sorted(fake.read) == ["c2", "c4"]


def test_full_reads_every_new_call_whatever_the_rule_says(store, batches):
    seed(store, 6)
    pattern(store, mode="full")
    rule_matched(store, 1, ["c2"])
    fake = batches()

    run(store)

    assert sorted(fake.read) == [f"c{i + 1}" for i in range(6)]


def test_a_call_already_paid_for_is_not_read_again(store, batches):
    """`pattern_labels` is the record of what has been read. A model is not asked the same
    question twice, whichever run asked it the first time — the wizard's sample included."""
    seed(store, 6)
    p = pattern(store)
    already_read(store, p, ["c1", "c2", "c3"])
    fake = batches()

    run(store)

    assert sorted(fake.read) == ["c4", "c5", "c6"]


def test_the_newest_calls_are_read_first(store, batches):
    """A capped run reads part of what it was given, so the part it reads has to be the
    part somebody is about to look at."""
    seed(store, 5)
    pattern(store)
    fake = batches()

    run(store)

    assert fake.read == ["c5", "c4", "c3", "c2", "c1"]


def test_a_call_with_nothing_said_on_it_is_not_read(store, batches):
    """Paying a model to say it cannot read an empty transcript buys a label that means
    "we do not know" wearing one that means "no"."""
    seed(store, 3)
    seed(store, 2, first=4, transcript="   ")
    pattern(store)
    fake = batches()

    run(store)

    assert sorted(fake.read) == ["c1", "c2", "c3"]


def test_a_pattern_scoped_to_an_assistant_reads_only_that_assistants_calls(store, batches):
    seed(store, 3, assistant_id="a1")
    seed(store, 2, first=4, assistant_id="a2")
    pattern(store, assistant_ids=json.dumps(["a2"]))
    fake = batches()

    run(store)

    assert sorted(fake.read) == ["c4", "c5"]


def test_a_free_pattern_is_never_read(store, batches):
    """The whole point of free mode: a rule decides it, and nothing here costs anything."""
    seed(store, 4)
    pattern(store, mode="free")
    fake = batches()

    result = run(store)

    assert fake.sent == []
    assert answer(result)["patterns"] == []
    assert answer(result)["usd"] == 0.0


def test_an_org_is_read_by_its_own_patterns_only(store, batches):
    seed(store, 3, org=1)
    seed(store, 2, first=4, org=2)
    pattern(store, org_id=2)
    fake = batches()

    run(store, org=2)

    assert sorted(fake.read) == ["c4", "c5"]


# --- what the verdicts do -------------------------------------------------------------


def test_a_confirmed_call_becomes_a_match_the_model_signed(store, batches):
    seed(store, 2)
    p = pattern(store, mode="hybrid")
    rule_matched(store, p, ["c1", "c2"])
    batches(answer=all_match)

    run(store)

    got = rows(store, "SELECT call_id, source FROM pattern_matches ORDER BY call_id, source")
    assert got == [
        {"call_id": "c1", "source": "llm"},
        {"call_id": "c1", "source": "rule"},
        {"call_id": "c2", "source": "llm"},
        {"call_id": "c2", "source": "rule"},
    ]


def test_a_rejected_call_loses_the_rules_row(store, batches):
    """What makes a hybrid confirmation worth paying for. Left in place, the count beside
    the pattern would read the same before and after the model was asked, and the money
    would have bought a number that did not move."""
    seed(store, 2)
    p = pattern(store, mode="hybrid")
    rule_matched(store, p, ["c1", "c2"])
    batches(answer=none_match)

    run(store)

    assert rows(store, "SELECT call_id FROM pattern_matches") == []
    assert [r["llm_match"] for r in rows(store, "SELECT llm_match FROM pattern_labels")] == [0, 0]


def test_what_the_model_said_is_stored_against_the_pattern(store, batches):
    seed(store, 2)
    p = pattern(store)
    batches()

    result = run(store)

    labels = rows(store, "SELECT pattern_id, call_id, llm_match FROM pattern_labels ORDER BY call_id")
    assert labels == [
        {"pattern_id": p, "call_id": "c1", "llm_match": 1},
        {"pattern_id": p, "call_id": "c2", "llm_match": 1},
    ]
    assert answer(result)["patterns"][0]["matched"] == 2


# --- rows that cannot run -------------------------------------------------------------


@pytest.mark.parametrize(
    "broken",
    [
        {"daily_cap_usd": None},
        {"daily_cap_usd": 0.0},
        {"model": None},
        {"plan": None},
        {"criterion": "  "},
    ],
)
def test_a_pattern_missing_what_it_needs_is_skipped_not_guessed_at(store, batches, broken):
    """A cap that can be left out is a cap that gets left out, and a model that was never
    chosen is not one to pick on the row's behalf."""
    seed(store, 3)
    pattern(store, **broken)
    fake = batches()

    result = run(store)

    assert result.exit_code == 0
    assert fake.sent == []
    assert "skipped" in result.output


def test_a_hybrid_pattern_with_no_rule_is_skipped(store, batches):
    """Without a rule there is nothing to prefilter, and reading the whole org would be
    full mode bought by accident."""
    seed(store, 3)
    pattern(store, mode="hybrid", rule=None)
    fake = batches()

    result = run(store)

    assert fake.sent == []
    assert "hybrid with no rule" in result.output


def test_a_pattern_with_nothing_new_costs_nothing(store, batches):
    seed(store, 2)
    p = pattern(store)
    already_read(store, p, ["c1", "c2"])
    fake = batches()

    result = run(store)

    assert fake.sent == []
    assert answer(result)["usd"] == 0.0
    assert answer(result)["patterns"][0]["read"] == 0


# --- failure --------------------------------------------------------------------------


def test_a_pattern_that_falls_over_does_not_take_the_spend_with_it(store, batches):
    """The engine books this run's cost from its last line. A traceback out of here would
    be money spent and no line to book it from, so a failure is reported rather than
    raised — and the patterns after it are still read."""
    seed(store, 2)
    first = pattern(store, name="one")
    second = pattern(store, name="two")
    fake = batches(usd=0.05)

    def explode(job, batch):
        if job.pattern_id == first:
            raise RuntimeError("the provider hung up")
        return fake(job, batch)

    import graphify_brain.label as mod

    mod.call_batch = explode
    try:
        result = run(store)
    finally:
        mod.call_batch = fake

    assert result.exit_code == 0
    got = answer(result)
    assert got["usd"] == pytest.approx(0.05)
    assert got["patterns"][0]["error"] == "RuntimeError: the provider hung up"
    assert got["patterns"][1]["read"] == 2


def test_a_request_without_a_budget_is_refused(store):
    """D-8's one hard rule: a daily run has a cap or it does not run."""
    result = CliRunner().invoke(
        app, ["daily", "--db", str(store)], input=json.dumps({"org": 1}) + "\n"
    )

    assert result.exit_code == 1
    assert "missing max_usd" in result.output


def test_a_budget_of_nothing_is_refused(store):
    result = run(store, max_usd=0.0)

    assert result.exit_code == 1
    assert "max_usd must be a positive number" in result.output


# --- the contract with the engine -----------------------------------------------------


def test_the_columns_this_touches_are_the_ones_the_engine_makes():
    """The store above is a copy, and a copy can drift. These are the columns this module
    names, read back out of the engine's own migration."""
    schema = MIGRATION.read_text()

    for column in ("criterion", "plan", "rule", "model", "mode", "daily_cap_usd", "assistant_ids"):
        assert f"  {column} " in schema, column
    assert "CREATE TABLE pattern_matches" in schema
    assert "source     TEXT" in schema
