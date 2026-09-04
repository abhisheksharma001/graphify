"""Not one test in this file may call a model, and not one may run the real engine.

Two fakes. `synth.client` is replaced with `Never` for every test by an autouse fixture,
so a test that reached a provider fails on the reach rather than on the bill. And a fake
`graphify` goes on PATH: a script that reads the rule file it was handed and prints the
ids a test told it to print. That is what makes agreement assertable without a Rust
toolchain — and it is also how "must not execute anything returned by the model" is
checked, because the fake writes down the argv it was called with.
"""

import json
import os
import sqlite3
import stat
from pathlib import Path

import pytest
from typer.testing import CliRunner

from baml_client import types
from graphify_brain import cost
from graphify_brain import synth as synthesis
from graphify_brain.cli import app

ENGINE = Path(__file__).resolve().parents[2] / "engine" / "src"
MIGRATION = ENGINE.parent / "migrations" / "0001_init.sql"


class Never:
    def __getattr__(self, name):
        raise AssertionError(f"a test called {name}; no test may call a model")


@pytest.fixture(autouse=True)
def no_model(monkeypatch):
    monkeypatch.setattr(synthesis, "client", Never)


# --- the fake engine -------------------------------------------------------------------


@pytest.fixture
def engine(tmp_path, monkeypatch):
    """A `graphify` on PATH whose `rule-check` prints the ids a test chose.

    It reads the two files it was given, so a test can assert what the engine was actually
    shown: the rule as JSON, and the calls in `Subject` shape. It writes its argv down, so
    a test can assert nothing was ever handed to a shell.
    """
    home = tmp_path / "bin"
    home.mkdir()
    ids_file = home / "ids.txt"
    seen_file = home / "seen.json"
    ids_file.write_text("")
    fake = home / "graphify"
    fake.write_text(
        "#!/usr/bin/env python3\n"
        "import json, sys, pathlib\n"
        "home = pathlib.Path(__file__).parent\n"
        "argv = sys.argv[1:]\n"
        "seen = json.loads((home / 'seen.json').read_text()) if (home / 'seen.json').exists() else []\n"
        "rule = json.loads(pathlib.Path(argv[argv.index('--rule') + 1]).read_text())\n"
        "calls = json.loads(pathlib.Path(argv[argv.index('--calls') + 1]).read_text())\n"
        "seen.append({'argv': argv, 'rule': rule, 'calls': calls})\n"
        "(home / 'seen.json').write_text(json.dumps(seen))\n"
        "lines = [l for l in (home / 'ids.txt').read_text().splitlines() if l.strip()]\n"
        "if lines and lines[0] == 'REFUSE':\n"
        "    sys.stderr.write('rule.json: unknown key min_turns\\n'); sys.exit(1)\n"
        "print('\\n'.join(lines[1:] if lines and lines[0] == 'ROUND' else lines))\n"
    )
    fake.chmod(fake.stat().st_mode | stat.S_IEXEC)
    monkeypatch.setenv("PATH", f"{home}{os.pathsep}{os.environ['PATH']}")

    class Fake:
        def matches(self, ids):
            ids_file.write_text("\n".join(ids))

        def refuses(self):
            ids_file.write_text("REFUSE")

        @property
        def runs(self):
            return json.loads(seen_file.read_text()) if seen_file.exists() else []

    return Fake()


# --- the fake model --------------------------------------------------------------------


def a_rule(**over):
    fields = {
        "any_phrases": ["talk to a person", "get me a human"],
        "regex": [],
        "speaker": "user",
        "ended_reasons": [],
        "ended_groups": [],
        "tool_called": [],
        "tool_not_called": [],
        "tool_failed": None,
        "transferred": None,
        "min_duration_s": None,
        "max_duration_s": None,
    }
    return types.Rule(**(fields | over))


def a_refinement(rule=None):
    return types.Refinement(
        rule=rule or a_rule(),
        reason="Added the phrasing the missed calls shared. Still misses a caller who only sighs.",
    )


def a_synthesis(rule=None, title="Asked for a person"):
    return types.Synthesis(
        rule=rule or a_rule(),
        chart=types.Chart(kind=types.ChartKind.Line, title=title),
        reason="Keys on the two phrasings the matching calls share. Will miss a caller who only sighs.",
    )


class Model:
    """Stands in for both model calls, and charges what the test told it to."""

    def __init__(self, first, second, usd):
        self.first = first
        self.second = second
        self.usd = usd
        self.synthesized = []
        self.refined = []

    def synthesize(self, job):
        self.synthesized.append(job)
        return self.first, self.usd

    def refine(self, job, rule, disagreements):
        self.refined.append((rule, list(disagreements)))
        return self.second, self.usd


@pytest.fixture
def model(monkeypatch):
    def install(first=None, second=None, usd=0.0):
        fake = Model(first or a_synthesis(), second or a_refinement(), usd)
        monkeypatch.setattr(synthesis, "synthesize_rule", fake.synthesize)
        monkeypatch.setattr(synthesis, "refine_rule", fake.refine)
        return fake

    return install


# --- the database ----------------------------------------------------------------------


@pytest.fixture
def store(tmp_path):
    path = tmp_path / "graphify.db"
    conn = sqlite3.connect(path)
    conn.executescript(
        """
        CREATE TABLE calls (
          id TEXT PRIMARY KEY, transcript TEXT, ended_reason TEXT, ended_group TEXT,
          transferred INTEGER, duration_s REAL
        );
        CREATE TABLE tool_calls (call_id TEXT, name TEXT, failed INTEGER);
        CREATE TABLE patterns (
          id INTEGER PRIMARY KEY, org_id INTEGER, name TEXT, criterion TEXT,
          assistant_ids JSON, plan JSON, rule JSON, chart JSON, model TEXT,
          mode TEXT DEFAULT 'free', daily_cap_usd REAL DEFAULT 1.0, sample_size INTEGER,
          agreement REAL, created_at TEXT
        );
        CREATE TABLE pattern_labels (
          pattern_id INTEGER, call_id TEXT, llm_match INTEGER, rule_match INTEGER, evidence TEXT
        );
        """
    )
    conn.commit()
    conn.close()
    return path


def seed(store, n, **over):
    row = {
        "transcript": "user: get me a human",
        "ended_reason": "customer-ended-call",
        "ended_group": "customer",
        "transferred": 0,
        "duration_s": 44.0,
    } | over
    conn = sqlite3.connect(store)
    conn.executemany(
        "INSERT INTO calls (id, transcript, ended_reason, ended_group, transferred, duration_s) "
        "VALUES (?, ?, ?, ?, ?, ?)",
        [(f"c{i + 1}", *row.values()) for i in range(n)],
    )
    conn.commit()
    conn.close()
    return [f"c{i + 1}" for i in range(n)]


def a_plan():
    return types.Plan(
        rows=[types.PlanRow(if_="the caller asks for a person", then="counts as a match")],
        questions=[],
        confidence=1.0,
        expressible=True,
        reason="Nothing in the sentence reads two ways.",
    )


def labels_for(ids, matching):
    """A label per call: the ones in `matching` matched, the rest did not."""
    return [
        {
            "call_id": i,
            "match": i in matching,
            "evidence": "user: get me a human" if i in matching else "nothing in this call is about it",
        }
        for i in ids
    ]


def request(ids, matching, **over):
    body = {
        "criterion": "calls where the caller asked for a person",
        "plan": a_plan().model_dump(),
        "labels": labels_for(ids, matching),
        "model": "sonnet",
        "max_usd": 100.0,
        "org_id": 1,
        "name": "Asked for a person",
    } | over
    return json.dumps(body)


def run(store, stdin):
    return CliRunner().invoke(app, ["synthesize", "--db", str(store)], input=stdin)


def out(result):
    return json.loads(result.stdout.splitlines()[-1])


# --- the acceptance ---------------------------------------------------------------------


def test_forty_matches_of_which_the_rule_finds_thirty_eight_plus_two_others(store, engine, model):
    """The register's worked example, number for number. 250 calls, 40 labelled matches;
    the rule finds 38 of them and 2 calls nobody labelled. Agreement is 246/250 — and it
    is 246 because the 208 calls both sides said no to are calls they agree about."""
    ids = seed(store, 250)
    matched_by_model = set(ids[:40])
    engine.matches(ids[:38] + ids[40:42])
    model()

    result = run(store, request(ids, matched_by_model))

    assert result.exit_code == 0, result.stderr
    body = out(result)
    assert body["agreement"] == 0.984
    assert body["agreed"] == 246
    assert body["of"] == 250
    assert body["matched_by_rule"] == 40
    assert body["matched_by_model"] == 40


# --- nothing the model returns is executed ------------------------------------------------


def test_the_rule_reaches_the_engine_as_a_file_and_never_as_a_command(store, engine, model):
    """The step's must-not. The rule is written to a file and named by path in an argument
    list; there is no shell anywhere in the call, so a rule made of shell is a rule made of
    shell that matches nothing."""
    ids = seed(store, 4)
    hostile = a_rule(any_phrases=["'; rm -rf / #", "$(whoami)", "`id`"], regex=[".*"])
    engine.matches([])
    model(first=a_synthesis(rule=hostile))

    result = run(store, request(ids, {ids[0]}))

    assert result.exit_code == 0
    argv = engine.runs[0]["argv"]
    assert argv[0] == "rule-check"
    assert "--rule" in argv and "--calls" in argv
    # Not one character of the rule is in the command line — only the path of a file.
    assert not any("rm -rf" in a or "whoami" in a for a in argv)
    assert engine.runs[0]["rule"]["any_phrases"] == ["'; rm -rf / #", "$(whoami)", "`id`"]


def test_python_never_compiles_a_regex_the_model_wrote(store, engine, model):
    """A regex that would hang Python's engine for ever. `engine/src/rules.rs` compiles it
    with the `regex` crate, which has no backtracking; nothing here ever looks at it."""
    ids = seed(store, 3)
    engine.matches([])
    model(first=a_synthesis(rule=a_rule(regex=["(a+)+$"])))

    result = run(store, request(ids, {ids[0]}))

    assert result.exit_code == 0
    assert engine.runs[0]["rule"]["regex"] == ["(a+)+$"]


def test_a_rule_the_engine_refuses_is_the_engines_word_and_not_a_guess(store, engine, model):
    """The engine is the only thing that says what a rule means, so when it says no, its
    own message is what comes back."""
    ids = seed(store, 3)
    engine.refuses()
    model()

    result = run(store, request(ids, {ids[0]}))

    assert result.exit_code == 1
    assert "the engine refused the rule" in result.stderr
    assert "unknown key min_turns" in result.stderr


def test_the_null_scalars_are_dropped_rather_than_sent_as_null(store, engine, model):
    """`engine/src/rules.rs` reads a missing key as "do not ask" but cannot read a `null`
    into a `Vec`. The lists are always there; the scalars nobody set are gone."""
    ids = seed(store, 2)
    engine.matches([])
    model()

    run(store, request(ids, {ids[0]}))

    rule = engine.runs[0]["rule"]
    assert "tool_failed" not in rule and "transferred" not in rule
    assert rule["any_phrases"] and rule["ended_groups"] == []
    assert rule["speaker"] == "user"


# --- what the engine is shown -------------------------------------------------------------


def test_the_calls_reach_the_engine_in_the_shape_rule_check_reads(store, engine, model):
    ids = seed(store, 2)
    conn = sqlite3.connect(store)
    conn.execute("INSERT INTO tool_calls (call_id, name, failed) VALUES ('c1', 'bookAppointment', 1)")
    conn.commit()
    conn.close()
    engine.matches([])
    model()

    run(store, request(ids, {ids[0]}))

    calls = engine.runs[0]["calls"]
    assert set(calls[0]) == {"id", "transcript", "ended_reason", "ended_group", "transferred", "duration_s", "tool_calls"}
    assert calls[0]["tool_calls"] == [{"name": "bookAppointment", "failed": True}]
    assert calls[1]["tool_calls"] == []


def test_a_transfer_nobody_recorded_stays_unknown(store, engine, model):
    """Coercing it to false would make every call nobody recorded a transfer for match a
    rule that asks for `transferred: false`."""
    ids = seed(store, 2, transferred=None)
    engine.matches([])
    model()

    run(store, request(ids, {ids[0]}))

    assert engine.runs[0]["calls"][0]["transferred"] is None


# --- refinement ----------------------------------------------------------------------------


def test_agreement_above_the_floor_never_calls_refine(store, engine, model):
    ids = seed(store, 100)
    engine.matches(ids[:10])
    fake = model()

    result = run(store, request(ids, set(ids[:10])))

    assert out(result)["agreement"] == 1.0
    assert fake.refined == []
    assert out(result)["refined"] is False


def test_agreement_below_the_floor_refines_once_and_keeps_the_better_rule(store, engine, model, monkeypatch):
    """The floor is 0.85 and one call is made, not a loop. A wizard that quietly bought
    four rounds of refinement would be a wizard nobody priced."""
    ids = seed(store, 10)
    # First round: the rule finds three of the five, and two it should not have.
    engine.matches(ids[:3] + ids[5:7])
    better = a_refinement(a_rule(any_phrases=["fixed"]))
    fake = model(second=better)

    # The fake engine answers from a file, so the second `rule-check` has to be told what
    # to say before it runs. `refine_rule` is the last moment before it does.
    def refine(job, rule, disagreements):
        fake.refined.append((rule, list(disagreements)))
        engine.matches(ids[:5])
        return better, 0.0

    monkeypatch.setattr(synthesis, "refine_rule", refine)
    result = run(store, request(ids, set(ids[:5])))

    body = out(result)
    assert len(fake.refined) == 1
    assert body["refined"] is True
    assert body["agreement"] == 1.0
    assert engine.runs[-1]["rule"]["any_phrases"] == ["fixed"]
    # The refinement's own reason, not the first draft's. That text describes a rule
    # nobody is running any more.
    assert body["reason"].startswith("Added the phrasing")


def test_a_refinement_that_agrees_on_fewer_calls_is_thrown_away(store, engine, model, monkeypatch):
    """The model is not trusted to have improved anything. There is a number that says
    whether it did, and it was already paid for either way."""
    ids = seed(store, 10)
    engine.matches(ids[:3] + ids[5:7])
    worse = a_refinement(a_rule(any_phrases=["worse"]))
    fake = model(second=worse)

    def refine(job, rule, disagreements):
        fake.refined.append((rule, list(disagreements)))
        engine.matches([])
        return worse, 0.0

    monkeypatch.setattr(synthesis, "refine_rule", refine)
    result = run(store, request(ids, set(ids[:5])))

    body = out(result)
    assert len(fake.refined) == 1
    assert body["refined"] is False
    # The first rule's agreement, not the refinement's.
    assert body["agreement"] == 0.6
    assert body["reason"].startswith("Keys on the two phrasings")
    assert "refinement kept the original" in result.stderr
    stored = sqlite3.connect(store).execute("SELECT rule FROM patterns").fetchone()[0]
    assert json.loads(stored)["any_phrases"] == ["talk to a person", "get me a human"]


def test_refine_is_shown_at_most_thirty_disagreements(store, engine, model):
    """All of them would make the second call bigger than the first for a rule that is
    badly wrong — which is exactly where the extra tokens buy the least."""
    ids = seed(store, 100)
    engine.matches([])
    fake = model()

    run(store, request(ids, set(ids[:60])))

    _, disagreements = fake.refined[0]
    assert len(disagreements) == 60
    assert len(disagreements[: synthesis.MAX_DISAGREEMENTS]) == 30


def test_a_disagreement_carries_both_verdicts_and_the_quote(store, engine, model):
    ids = seed(store, 10)
    engine.matches([ids[9]])
    fake = model()

    run(store, request(ids, set(ids[:5])))

    _, disagreements = fake.refined[0]
    missed = [d for d in disagreements if d["labelled"] and not d["matched"]]
    caught = [d for d in disagreements if d["matched"] and not d["labelled"]]
    assert len(missed) == 5 and len(caught) == 1
    assert missed[0]["evidence"] == "user: get me a human"


# --- the price -----------------------------------------------------------------------------


def test_the_estimate_is_printed_before_anything(store, engine, model):
    ids = seed(store, 10)
    engine.matches([])
    model()

    result = run(store, request(ids, {ids[0]}))

    first = result.stdout.splitlines()[0]
    assert first.startswith("ESTIMATE ")
    assert float(first.split()[1]) > 0


def test_the_estimate_covers_the_refinement_too(store, engine):
    """The refinement is the worst case, and a cap is only a cap against the worst case."""
    conn = sqlite3.connect(store)
    conn.row_factory = sqlite3.Row
    seed(store, 10)
    job = synthesis.prepare(json.loads(request([f"c{i + 1}" for i in range(10)], {"c1"})), conn)

    assert synthesis.estimate(job) > synthesis._synthesize_usd(job)
    assert synthesis.estimate(job) == pytest.approx(
        synthesis._synthesize_usd(job) + synthesis._refine_usd(job)
    )


def test_a_job_that_cannot_fit_the_cap_sends_nothing(store, engine, model):
    ids = seed(store, 10)
    fake = model()

    result = run(store, request(ids, {ids[0]}, max_usd=0.000001))

    assert result.exit_code == 1
    assert fake.synthesized == []
    assert "the cap is" in result.stderr
    assert engine.runs == []


def test_the_cost_reported_is_what_the_calls_actually_charged(store, engine, model):
    ids = seed(store, 10)
    engine.matches(ids[:1])
    model(usd=0.0044)

    result = run(store, request(ids, {ids[0]}))

    assert out(result)["usd"] == 0.0044


# --- what is stored --------------------------------------------------------------------------


def test_the_pattern_row_is_written_in_free_mode(store, engine, model):
    """Free is the whole point of having got here: the model has been paid for and
    dismissed, and the rule runs for nothing. S-27's mode select is what changes that, in
    front of a person who can see the cap they are turning on."""
    ids = seed(store, 10)
    engine.matches(ids[:1])
    model()

    result = run(store, request(ids, {ids[0]}, assistant_ids=["a1", "a2"]))

    conn = sqlite3.connect(store)
    conn.row_factory = sqlite3.Row
    row = conn.execute("SELECT * FROM patterns").fetchone()
    assert row["id"] == out(result)["pattern_id"]
    assert row["mode"] == "free"
    assert row["org_id"] == 1
    assert row["name"] == "Asked for a person"
    assert row["criterion"] == "calls where the caller asked for a person"
    assert json.loads(row["assistant_ids"]) == ["a1", "a2"]
    assert json.loads(row["chart"]) == {"kind": "Line", "title": "Asked for a person"}
    assert row["sample_size"] == 10
    assert row["agreement"] == 1.0
    assert row["model"] == "sonnet"
    assert row["created_at"].endswith("Z")


def test_every_label_is_attached_to_the_pattern_with_both_verdicts(store, engine, model):
    """S-23 left `rule_match` NULL because there was no rule yet. It is filled in here,
    from the same `rule-check` that produced the agreement figure, so the two can never
    come to disagree."""
    ids = seed(store, 4)
    engine.matches(["c1", "c3"])
    model()

    run(store, request(ids, {"c1", "c2"}))

    conn = sqlite3.connect(store)
    rows = conn.execute(
        "SELECT call_id, llm_match, rule_match FROM pattern_labels ORDER BY call_id"
    ).fetchall()
    assert rows == [("c1", 1, 1), ("c2", 1, 0), ("c3", 0, 1), ("c4", 0, 0)]


# --- inputs refused ----------------------------------------------------------------------------


def test_labels_with_no_match_at_all_are_refused(store, engine, model):
    """A rule written from nothing but non-matches has nothing to key on, and the model
    will invent something that scores 1.0 by matching no call ever."""
    ids = seed(store, 4)
    fake = model()

    result = run(store, request(ids, set()))

    assert result.exit_code == 1
    assert fake.synthesized == []
    assert "no call was labelled a match" in result.stderr


def test_a_label_that_is_not_one_is_refused(store, engine, model):
    ids = seed(store, 2)
    body = json.loads(request(ids, {ids[0]}))
    body["labels"][0] = {"call_id": "c1", "match": "yes", "evidence": "x"}

    result = run(store, json.dumps(body))

    assert result.exit_code == 1
    assert "match must be true or false" in result.stderr


def test_a_labelled_call_that_is_not_in_the_database_is_refused(store, engine, model):
    seed(store, 2)
    fake = model()

    result = run(store, request(["c1", "c9"], {"c1"}))

    assert result.exit_code == 1
    assert fake.synthesized == []
    assert "not in the database" in result.stderr and "c9" in result.stderr


def test_the_same_call_labelled_twice_is_refused(store, engine, model):
    seed(store, 2)

    result = run(store, request(["c1", "c2", "c1"], {"c1"}))

    assert result.exit_code == 1
    assert "the same call twice" in result.stderr


def test_a_misspelled_field_is_named_rather_than_dropped(store, engine, model):
    ids = seed(store, 2)
    body = json.loads(request(ids, {ids[0]}))
    body["assistantids"] = ["a1"]

    result = run(store, json.dumps(body))

    assert result.exit_code == 1
    assert "has no field assistantids" in result.stderr


def test_a_model_with_no_published_price_is_refused(store, engine, model):
    ids = seed(store, 2)

    result = run(store, request(ids, {ids[0]}, model="haiku"))

    assert result.exit_code == 1
    assert "model must be one of gpt, opus, sonnet" in result.stderr


# --- the prompts, and the class the engine has to read -----------------------------------------


def rendered(name, **kwargs):
    from baml_client.sync_client import b

    body = getattr(b.request, name)(**kwargs).body.json()
    system = "\n".join(part["text"] for part in body.get("system", []))
    return body, system + "\n" + body["messages"][-1]["content"][0]["text"]


def test_the_rule_class_is_the_dsl_field_for_field():
    """`rule.baml` is what stops a model returning a key `engine/src/rules.rs` would
    refuse. If the two lists ever differ, this is where it shows."""
    assert set(types.Rule.model_fields) == {
        "any_phrases", "regex", "speaker", "ended_reasons", "ended_groups",
        "tool_called", "tool_not_called", "tool_failed", "transferred",
        "min_duration_s", "max_duration_s",
    }


def test_the_dsl_class_matches_the_dsl_the_model_is_described(monkeypatch):
    """The prompt describes the DSL in prose and the class constrains it. A field in one
    and not the other is a condition the model is told about and cannot return, or one it
    can return and was never told about."""
    from graphify_brain import plan as planning

    for field in types.Rule.model_fields:
        assert field in planning.DSL, field


def test_both_prompts_are_no_bigger_than_the_estimate_allows():
    empty = types.Plan(rows=[], questions=[], confidence=1.0, expressible=True, reason="")
    body, _ = rendered("SynthesizeRule", criterion="", plan=empty, labels=[], dsl="")
    assert len(json.dumps(body)) <= synthesis.SYNTHESIZE_PROMPT_CHARS

    body, _ = rendered("RefineRule", criterion="", plan=empty, rule=a_rule(), disagreements=[], dsl="")
    assert len(json.dumps(body)) <= synthesis.REFINE_PROMPT_CHARS


def test_baml_still_caps_the_output_where_the_estimate_says_it_does():
    body, _ = rendered("SynthesizeRule", criterion="x", plan=a_plan(), labels=[], dsl="")
    assert body["max_tokens"] == synthesis.MAX_OUTPUT_TOKENS


def test_the_synthesize_prompt_carries_the_evidence_and_the_dsl():
    from graphify_brain import plan as planning

    _, text = rendered(
        "SynthesizeRule",
        criterion="calls where the caller asked for a person",
        plan=a_plan(),
        labels=[
            types.LabelForRule(match=True, evidence="user: get me a human"),
            types.LabelForRule(match=False, evidence="user: what are your hours"),
        ],
        dsl=planning.DSL,
    )

    assert "calls where the caller asked for a person" in text
    assert "MATCH" in text and "no match" in text
    assert "user: get me a human" in text and "user: what are your hours" in text
    assert "tool_not_called" in text and "not expressible" in text


def test_the_refine_prompt_carries_the_rule_and_both_verdicts():
    from graphify_brain import plan as planning

    _, text = rendered(
        "RefineRule",
        criterion="calls where the caller asked for a person",
        plan=a_plan(),
        rule=a_rule(),
        disagreements=[
            types.Disagreement(labelled=True, matched=False, evidence="user: is there a person there"),
        ],
        dsl=planning.DSL,
    )

    assert "get me a human" in text
    assert "labelled MATCH, rule said no match" in text
    assert "user: is there a person there" in text
    assert "tool_not_called" in text


def test_the_columns_this_writes_are_the_ones_the_engine_makes():
    schema = MIGRATION.read_text()

    for column in ("org_id", "name", "criterion", "assistant_ids", "plan", "rule", "chart",
                   "model", "mode", "daily_cap_usd", "sample_size", "agreement", "created_at"):
        assert f"  {column} " in schema, column
    assert "CREATE TABLE patterns" in schema


def test_the_call_shape_this_sends_is_the_one_the_engine_will_read():
    """`rules.rs` puts `deny_unknown_fields` on `Subject`, so a key this invents is not
    ignored — every call is refused and the whole run dies. The two lists have to be the
    same list, and reading the Rust is the only way to know that they are.

    Checked here rather than by running the engine, because the brain's CI job has no Rust
    toolchain. The real round trip was done by hand against `target/debug/graphify` when
    this was written; this is what keeps it true.
    """
    rust = (ENGINE / "rules.rs").read_text()
    subject = rust.split("pub struct Subject {", 1)[1].split("}", 1)[0]
    fields = {line.split(":")[0].replace("pub ", "").strip() for line in subject.splitlines() if "pub " in line}

    assert fields == {"id", "transcript", "ended_reason", "ended_group", "transferred", "duration_s", "tool_calls"}

    tool = rust.split("pub struct Tool {", 1)[1].split("}", 1)[0]
    assert {line.split(":")[0].replace("pub ", "").strip() for line in tool.splitlines() if "pub " in line} == {"name", "failed"}


def test_the_rule_shape_this_sends_is_the_one_the_engine_will_read():
    """Same reason, the other direction: `Rule` is `deny_unknown_fields` too, so a field
    `rule.baml` has and `rules.rs` does not is a rule the engine throws out entire."""
    rust = (ENGINE / "rules.rs").read_text()
    body = rust.split("pub struct Rule {", 1)[1].split("\n}", 1)[0]
    fields = {line.split(":")[0].replace("pub ", "").strip() for line in body.splitlines() if "pub " in line}

    assert fields == set(types.Rule.model_fields)

