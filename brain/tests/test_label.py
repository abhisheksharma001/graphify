"""Not one test in this file may call a model, and every test here is about money.

`label` is the first command that spends at a scale a person would notice, so the fakes
are stricter than the ones in `test_plan.py`. `label.client` is replaced with `Never` for
every test in the file, by a fixture nobody has to remember — so a test that reached a
provider would fail on the reach, not on the bill. `label.call_batch` is replaced with
`Batches`, which counts what was sent and charges what the test told it to; that is what
makes "exactly three model calls" and "stopped at the cap" assertable at all.
"""

import json
import sqlite3
import threading
from pathlib import Path

import pytest
from typer.testing import CliRunner

from baml_client import types
from graphify_brain import cost
from graphify_brain import label as labelling
from graphify_brain.cli import app

MIGRATION = Path(__file__).resolve().parents[2] / "engine" / "migrations" / "0001_init.sql"

#: A transcript long enough that a batch of them costs a measurable fraction of a cent,
#: so the cap tests are about arithmetic and not about floating-point dust.
LINES = "\n".join(
    [
        "user: hi, is anyone actually there",
        "bot: I can help with that. What are you calling about?",
        "user: I want to talk to a person please",
        "bot: Let me put you through.",
    ]
)

#: Three real lines out of `LINES`. Tests that need to tell one label from another use these
#: rather than a made-up string, because S-68 replaces evidence a transcript does not
#: contain — a placeholder quote would be testing that replacement instead of attachment.
QUOTES = [
    "user: hi, is anyone actually there",
    "user: I want to talk to a person please",
    "bot: Let me put you through.",
]


class Never:
    """Reaching for anything on this is reaching for a model. That is the failure."""

    def __getattr__(self, name):
        raise AssertionError(f"a test called {name}; no test may call a model")


@pytest.fixture(autouse=True)
def no_model(monkeypatch):
    """Every test, whether it remembers to ask or not."""
    monkeypatch.setattr(labelling, "client", Never)


class Batches:
    """Stands in for `call_batch`: remembers every batch, and charges a fixed price.

    The price is the test's to choose, because the whole point of the cap is what happens
    when a batch costs what it costs and the total gets close to a number.
    """

    def __init__(self, usd, answer):
        self.sent = []
        self.usd = usd
        self.answer = answer
        self.at_once = 0
        self.most_at_once = 0
        self.lock = threading.Lock()

    def __call__(self, job, batch):
        with self.lock:
            self.sent.append(list(batch))
            self.at_once += 1
            self.most_at_once = max(self.most_at_once, self.at_once)
        try:
            return self.answer(batch), self.usd
        finally:
            with self.lock:
                self.at_once -= 1


def all_match(batch):
    return [types.Label(n=i + 1, match=True, evidence="user: I want to talk to a person please") for i in range(len(batch))]


@pytest.fixture
def batches(monkeypatch):
    def install(usd=0.0, answer=all_match):
        fake = Batches(usd, answer)
        monkeypatch.setattr(labelling, "call_batch", fake)
        return fake

    return install


@pytest.fixture
def store(tmp_path):
    """An engine-shaped database. `seed` puts calls in it.

    Built by hand rather than from the engine's migration, as `test_db.py` does — but the
    columns are the ones this module SELECTs, so `test_the_columns_this_reads_are_the_ones
    _the_engine_makes` checks the copy against the original.
    """
    path = tmp_path / "graphify.db"
    conn = sqlite3.connect(path)
    conn.executescript(
        """
        CREATE TABLE calls (
          id TEXT PRIMARY KEY, transcript TEXT, duration_s REAL, ended_reason TEXT,
          ended_group TEXT, transferred INTEGER, tool_calls INTEGER, tool_failures INTEGER
        );
        CREATE TABLE tool_calls (call_id TEXT, name TEXT, seconds_from_start REAL, failed INTEGER);
        CREATE TABLE pattern_labels (
          pattern_id INTEGER, call_id TEXT, llm_match INTEGER, rule_match INTEGER, evidence TEXT
        );
        """
    )
    conn.commit()
    conn.close()
    return path


def seed(store, n, **over):
    """`n` calls, `c1`…`cn`, all alike unless a test says otherwise."""
    row = {
        "transcript": LINES,
        "duration_s": 92.0,
        "ended_reason": "customer-ended-call",
        "ended_group": "customer",
        "transferred": 0,
        "tool_calls": 0,
        "tool_failures": 0,
    } | over
    conn = sqlite3.connect(store)
    conn.executemany(
        "INSERT INTO calls (id, transcript, duration_s, ended_reason, ended_group, transferred, "
        "tool_calls, tool_failures) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
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


def request(ids, **over):
    body = {
        "criterion": "calls where the caller asked for a person",
        "plan": a_plan().model_dump(),
        "call_ids": ids,
        "model": "sonnet",
        "max_usd": 100.0,
    } | over
    return json.dumps(body)


def run(store, stdin, *flags):
    """The command as the engine runs it: one line of JSON, then whatever comes next."""
    return CliRunner().invoke(app, ["label", "--db", str(store), *flags], input=stdin)


def priced(store, ids, **over):
    """What this job would be estimated at, per batch and in total, without running it."""
    conn = sqlite3.connect(store)
    conn.row_factory = sqlite3.Row
    job = labelling.prepare(json.loads(request(ids, **over)), conn)
    return labelling.estimate(job), [labelling.batch_usd(job, b) for b in labelling.batches(job)]


# --- the acceptance -------------------------------------------------------------------


def test_forty_five_calls_at_twenty_a_batch_are_exactly_three_model_calls(store, batches):
    """The register's first acceptance. Batching is what makes labelling affordable at
    all — one call per transcript would be forty-five round trips and forty-five copies
    of the plan paid for."""
    ids = seed(store, 45)
    fake = batches()

    result = run(store, request(ids) + "\nGO\n")

    assert result.exit_code == 0
    assert len(fake.sent) == 3
    # Sorted, because three batches in flight come back in whatever order they come back
    # in. What must not vary is the cut: twenty, twenty, and the five that are left.
    assert sorted(len(b) for b in fake.sent) == [5, 20, 20]
    out = json.loads(result.stdout.splitlines()[-1])
    assert out["batches"] == 3
    # Every call labelled, each against its own id: the proof that a number the model
    # answered with in one batch was not read against another batch's calls.
    assert [x["call_id"] for x in out["labels"]] == ids


def test_stdin_that_never_says_go_reads_nothing(store, batches):
    """The register's second acceptance, and the spec's must-never: no model call without
    an explicit go. Silence is not a go."""
    ids = seed(store, 45)
    fake = batches()

    result = run(store, request(ids) + "\n")

    assert result.exit_code == 0
    assert fake.sent == []
    out = json.loads(result.stdout.splitlines()[-1])
    assert out["stopped"] == "declined"
    assert out["usd"] == 0.0
    assert out["not_reached"] == ids


def test_anything_but_go_is_not_a_go(store, batches):
    """`y`, `yes`, an empty line, a typo. The word is `GO`."""
    ids = seed(store, 5)

    for answer in ("y\n", "yes\n", "\n", "go go\n", "Go\n"):
        fake = batches()
        result = run(store, request(ids) + "\n" + answer)
        assert fake.sent == [], answer
        assert json.loads(result.stdout.splitlines()[-1])["stopped"] == "declined"


# --- the price is shown first ---------------------------------------------------------


def test_the_estimate_is_printed_before_anything_is_read(store, batches):
    ids = seed(store, 45)
    batches()

    result = run(store, request(ids) + "\nGO\n")

    first = result.stdout.splitlines()[0]
    assert first.startswith("ESTIMATE ")
    assert float(first.split()[1]) > 0


def test_the_estimate_is_printed_even_when_nobody_will_be_asked(store, batches):
    """`--yes` skips the wait, not the showing. The must-never is a cost *shown* and a go;
    a run that spends without ever naming a price is the thing it forbids."""
    ids = seed(store, 5)
    fake = batches()

    result = run(store, request(ids), "--yes")

    assert result.stdout.splitlines()[0].startswith("ESTIMATE ")
    assert len(fake.sent) == 1


def test_the_estimate_prices_output_at_what_baml_will_actually_allow(store):
    """The output half of the estimate is a bound, not a guess: BAML sends `max_tokens`,
    so a batch cannot return more than that however much it wants to."""
    ids = seed(store, 1)
    total, per_batch = priced(store, ids)

    floor = cost.estimate(0, labelling.MAX_OUTPUT_TOKENS, "sonnet")
    assert total == per_batch[0]
    assert total > floor


def test_the_estimate_grows_with_the_transcripts_it_will_read(store):
    ids = seed(store, 4)
    small, _ = priced(store, ids[:1])
    large, _ = priced(store, ids)

    assert large > small


# --- the cap --------------------------------------------------------------------------


def test_a_cap_below_one_batch_reads_nothing(store, batches):
    """The must-not, at its sharpest. A cap that cannot afford the first batch stops
    before the first batch, not after it."""
    ids = seed(store, 9)
    _, per_batch = priced(store, ids, batch_size=3)
    fake = batches(usd=per_batch[0])

    result = run(store, request(ids, batch_size=3, max_usd=per_batch[0] / 2) + "\nGO\n")

    assert fake.sent == []
    out = json.loads(result.stdout.splitlines()[-1])
    assert out["stopped"] == "cap"
    assert out["usd"] == 0.0
    assert out["not_reached"] == ids
    assert "cap reached" in result.stderr


def test_a_cap_that_affords_two_of_three_batches_sends_two(store, batches):
    ids = seed(store, 9)
    _, per_batch = priced(store, ids, batch_size=3)
    cap = per_batch[0] + per_batch[1] + per_batch[2] / 2
    fake = batches(usd=per_batch[0])

    result = run(store, request(ids, batch_size=3, max_usd=cap) + "\nGO\n")

    assert len(fake.sent) == 2
    out = json.loads(result.stdout.splitlines()[-1])
    assert out["stopped"] == "cap"
    assert out["batches"] == 2
    assert out["not_reached"] == ids[6:]
    assert len(out["labels"]) == 6


def test_what_is_spent_never_passes_the_cap(store, batches):
    """The step's must-not, asserted rather than argued. Each batch is charged exactly
    what it was estimated at — the worst case the estimate allows for — and the total
    still comes in under the number."""
    ids = seed(store, 9)
    _, per_batch = priced(store, ids, batch_size=3)

    for fraction in (0.4, 0.9, 1.6, 2.4, 3.0, 5.0):
        cap = sum(per_batch) * fraction / 3
        fake = batches(usd=per_batch[0])
        result = run(store, request(ids, batch_size=3, max_usd=cap) + "\nGO\n")
        spent = json.loads(result.stdout.splitlines()[-1])["usd"]
        assert spent <= cap, f"{fraction}: spent {spent} against a cap of {cap}"
        assert len(fake.sent) * per_batch[0] == pytest.approx(spent, rel=1e-6)


def test_a_cap_is_required(store, batches):
    """No default. A cap a caller can leave out is a cap that gets left out."""
    ids = seed(store, 2)
    body = json.loads(request(ids))
    del body["max_usd"]

    result = run(store, json.dumps(body) + "\nGO\n")

    assert result.exit_code == 1
    assert "missing max_usd" in result.stderr


def test_a_cap_of_zero_is_refused(store, batches):
    ids = seed(store, 2)

    result = run(store, request(ids, max_usd=0) + "\nGO\n")

    assert result.exit_code == 1
    assert "max_usd must be a positive number" in result.stderr


# --- progress and concurrency ---------------------------------------------------------


def test_progress_counts_batches_on_stderr(store, batches):
    """`PROGRESS n/m`, the engine ↔ brain contract. The engine streams these into
    `jobs.log` and the wizard draws a bar from them."""
    ids = seed(store, 45)
    batches()

    result = run(store, request(ids, batch_size=5) + "\nGO\n")

    lines = [x for x in result.stderr.splitlines() if x.startswith("PROGRESS")]
    assert lines == ["PROGRESS 3/9", "PROGRESS 6/9", "PROGRESS 9/9"]


def test_at_most_three_batches_are_in_flight(store, batches):
    ids = seed(store, 45)
    fake = batches()

    run(store, request(ids, batch_size=2) + "\nGO\n")

    assert len(fake.sent) == 23
    assert fake.most_at_once <= labelling.CONCURRENCY


# --- what comes back ------------------------------------------------------------------


def test_a_label_is_attached_to_the_call_its_number_meant(store, batches):
    ids = seed(store, 3)
    batches(answer=lambda b: [types.Label(n=2, match=True, evidence=QUOTES[1])])

    result = run(store, request(ids) + "\nGO\n")

    out = json.loads(result.stdout.splitlines()[-1])
    assert out["labels"] == [{"call_id": "c2", "match": True, "evidence": QUOTES[1]}]
    assert sorted(out["no_label"]) == ["c1", "c3"]


def test_a_label_for_a_number_that_was_not_in_the_batch_is_dropped(store, batches):
    """A model that answers about call 9 of a batch of 3 is answering about nothing. The
    alternative is attaching that judgement to some call it was not about."""
    ids = seed(store, 3)
    batches(answer=lambda b: [types.Label(n=9, match=True, evidence="from nowhere")])

    result = run(store, request(ids) + "\nGO\n")

    out = json.loads(result.stdout.splitlines()[-1])
    assert out["labels"] == []
    assert sorted(out["no_label"]) == ids


def test_the_same_number_twice_labels_the_call_once(store, batches):
    ids = seed(store, 2)
    batches(
        answer=lambda b: [
            types.Label(n=1, match=True, evidence=QUOTES[0]),
            types.Label(n=1, match=False, evidence=QUOTES[2]),
        ]
    )

    result = run(store, request(ids) + "\nGO\n")

    out = json.loads(result.stdout.splitlines()[-1])
    assert out["labels"] == [{"call_id": "c1", "match": True, "evidence": QUOTES[0]}]
    assert out["no_label"] == ["c2"]


# --- a quote that is not in the transcript (S-68) ---------------------------------------


def stored(store):
    """Every `pattern_labels` row, in call order."""
    return (
        sqlite3.connect(store)
        .execute("SELECT call_id, llm_match, evidence FROM pattern_labels ORDER BY call_id")
        .fetchall()
    )


def test_a_quote_the_transcript_does_not_contain_is_not_stored_as_evidence(store, batches):
    """The defect. `label.baml` says "Never quote a line that is not in the transcript you
    were given" and until S-68 nothing held the model to it, so an invented sentence reached
    `pattern_labels`, the call drawer, and `SynthesizeRule` — which is told the quotes are
    what people actually said and writes a rule that then runs unattended forever."""
    ids = seed(store, 1)
    batches(answer=lambda b: [types.Label(n=1, match=True, evidence="user: cancel my policy")])

    run(store, request(ids, pattern_id=7) + "\nGO\n")

    assert stored(store) == [("c1", 1, labelling.NOT_IN_TRANSCRIPT)]


def test_the_judgement_survives_the_quote_that_did_not(store, batches):
    """What is thrown away is the sentence, not the label. The transcript was read and the
    batch was paid for; dropping the label would spend the money and buy nothing."""
    ids = seed(store, 2)
    batches(
        answer=lambda b: [
            types.Label(n=1, match=True, evidence="user: cancel my policy"),
            types.Label(n=2, match=False, evidence="user: cancel my policy"),
        ]
    )

    result = run(store, request(ids, pattern_id=7) + "\nGO\n")

    assert [(i, m) for i, m, _ in stored(store)] == [("c1", 1), ("c2", 0)]
    assert json.loads(result.stdout.splitlines()[-1])["no_label"] == []


def test_a_quote_that_differs_only_in_spacing_or_case_is_kept(store, batches):
    """A model that normalised a double space or a capital letter in an otherwise honest
    quote has invented nothing. A check that cried wolf at that would be turned off."""
    ids = seed(store, 1)
    batches(answer=lambda b: [types.Label(n=1, match=True, evidence="I want to  TALK to a Person")])

    run(store, request(ids, pattern_id=7) + "\nGO\n")

    assert stored(store) == [("c1", 1, "I want to  TALK to a Person")]


def test_the_prompts_own_no_quote_sentence_is_not_an_invention(store, batches):
    """`label.baml` asks for this sentence in place of a quote when a call does not match
    and nothing in it came close. It is not a quote, so it must not be reported as a bad
    one — the analyst reading the drawer is owed the difference between "nothing was said
    about this" and "the model made something up"."""
    ids = seed(store, 1)
    batches(answer=lambda b: [types.Label(n=1, match=False, evidence=labelling.NO_QUOTE)])

    run(store, request(ids, pattern_id=7) + "\nGO\n")

    assert stored(store) == [("c1", 0, labelling.NO_QUOTE)]


def test_the_no_quote_sentence_is_the_one_the_prompt_asks_for():
    """The pin. `NO_QUOTE` is only safe to let through because the prompt asks for exactly
    it; a reworded prompt would quietly turn every non-match's evidence into a replacement
    notice. Same guard `FIXED_PROMPT_CHARS` has, for the same reason."""
    prompt = (Path(__file__).resolve().parents[1] / "baml_src" / "label.baml").read_text()

    assert f'"{labelling.NO_QUOTE}"' in prompt


def test_the_log_says_how_many_quotes_were_replaced(store, batches):
    """A silent repair is how this lived this long. The analyst paying for the run can read
    in its log that the model invented two of three quotes."""
    ids = seed(store, 3)
    batches(
        answer=lambda b: [
            types.Label(n=1, match=True, evidence=QUOTES[0]),
            types.Label(n=2, match=True, evidence="user: cancel my policy"),
            types.Label(n=3, match=True, evidence="user: I have not called before"),
        ]
    )

    result = run(store, request(ids, pattern_id=7) + "\nGO\n")

    assert "2 of 3 quotes were not in the transcript they were about" in result.stderr


def test_a_run_whose_quotes_were_all_real_says_nothing_about_it(store, batches):
    ids = seed(store, 2)

    result = run(store, request(ids, pattern_id=7) + "\nGO\n")

    assert "quotes were not in the transcript" not in result.stderr


def test_every_call_asked_about_is_in_exactly_one_list(store, batches):
    """Four lists, four causes: labelled, nothing to read, no answer, never reached. A
    call that fell out of all of them would be one nobody could account for."""
    ids = seed(store, 4)
    sqlite3.connect(store).executescript("UPDATE calls SET transcript = NULL WHERE id = 'c4'")
    batches(answer=lambda b: [types.Label(n=1, match=True, evidence="yes")])

    result = run(store, request(ids) + "\nGO\n")

    out = json.loads(result.stdout.splitlines()[-1])
    seen = [x["call_id"] for x in out["labels"]] + out["no_transcript"] + out["no_label"] + out["not_reached"]
    assert sorted(seen) == sorted(ids)


def test_the_actual_cost_is_reported_not_the_estimate(store, batches):
    ids = seed(store, 4)
    batches(usd=0.0123)

    result = run(store, request(ids) + "\nGO\n")

    lines = result.stdout.splitlines()
    assert json.loads(lines[-1])["usd"] == 0.0123
    assert float(lines[0].split()[1]) != 0.0123


# --- what the model is shown ----------------------------------------------------------


def test_a_call_with_no_transcript_is_never_sent(store, batches):
    """There is nothing to read, and paying a model to say so buys a label that means "we
    do not know" wearing the clothes of one that means "no"."""
    ids = seed(store, 3)
    conn = sqlite3.connect(store)
    conn.execute("UPDATE calls SET transcript = '   ' WHERE id = 'c2'")
    conn.commit()
    fake = batches()

    result = run(store, request(ids) + "\nGO\n")

    assert [c.id for c in fake.sent[0]] == ["c1", "c3"]
    assert json.loads(result.stdout.splitlines()[-1])["no_transcript"] == ["c2"]


def test_a_duration_nobody_recorded_is_a_dash_and_not_a_zero(store, batches):
    """The spec's must-never. A model told a call lasted 0 seconds will reason about a
    call that never connected."""
    ids = seed(store, 1, duration_s=None, transferred=None, tool_calls=None, tool_failures=None)
    fake = batches()

    run(store, request(ids) + "\nGO\n")

    facts = fake.sent[0][0].facts
    assert "lasted —" in facts
    assert "transferred —" in facts
    assert "tools run —" in facts
    assert "0s" not in facts


def test_no_tools_and_no_record_of_tools_read_differently(store, batches):
    """`tool_calls = 0` is a call where nothing ran; `NULL` is a call nobody looked at.
    A rule about `tool_not_called` turns on the difference."""
    seed(store, 1, tool_calls=0)
    fake = batches()
    run(store, request(["c1"]) + "\nGO\n")
    assert "tools run none" in fake.sent[0][0].facts

    conn = sqlite3.connect(store)
    conn.execute("UPDATE calls SET tool_calls = NULL")
    conn.commit()
    fake = batches()
    run(store, request(["c1"]) + "\nGO\n")
    assert "tools run —" in fake.sent[0][0].facts


def test_the_facts_line_names_the_tools_that_ran_and_the_ones_that_failed(store, batches):
    """A plan row can be about something nobody says out loud — a booking tool that
    failed, a call that ended in an error. A labeller that could not see those would be
    guessing at exactly the calls S-24 measures its rule against."""
    seed(store, 1, tool_calls=2, tool_failures=1)
    conn = sqlite3.connect(store)
    conn.executemany(
        "INSERT INTO tool_calls (call_id, name, failed) VALUES (?, ?, ?)",
        [("c1", "bookAppointment", 0), ("c1", "transferCall", 1)],
    )
    conn.commit()
    fake = batches()

    run(store, request(["c1"]) + "\nGO\n")

    facts = fake.sent[0][0].facts
    assert "tools run bookAppointment, transferCall" in facts
    assert "tools failed transferCall" in facts
    assert "ended customer-ended-call (customer)" in facts


# --- inputs refused, with nothing spent -----------------------------------------------


def test_a_call_id_that_is_not_in_the_database_is_refused(store, batches):
    """Not skipped. S-24 divides by the number of labels to get an agreement figure, and
    labelling eight of the nine calls somebody asked about makes that figure quietly
    wrong about which calls it describes."""
    seed(store, 3)
    fake = batches()

    result = run(store, request(["c1", "c9", "c2"]) + "\nGO\n")

    assert result.exit_code == 1
    assert fake.sent == []
    assert "not in the database" in result.stderr and "c9" in result.stderr


def test_the_same_call_twice_is_refused(store, batches):
    """It would be read twice, paid for twice, and land in `pattern_labels` twice with
    two answers to the same question."""
    seed(store, 2)
    fake = batches()

    result = run(store, request(["c1", "c2", "c1"]) + "\nGO\n")

    assert result.exit_code == 1
    assert fake.sent == []
    assert "repeats an id" in result.stderr


def test_a_model_with_no_published_price_is_refused(store, batches):
    """An unpriced model is not a free one: it is one whose spend the cap cannot count."""
    ids = seed(store, 2)

    result = run(store, request(ids, model="haiku") + "\nGO\n")

    assert result.exit_code == 1
    assert "model must be one of gpt, opus, sonnet" in result.stderr


def test_a_batch_bigger_than_twenty_is_refused(store, batches):
    """D-3: twenty at a time. A hundred transcripts in one call is one answer nobody can
    check, in the wrong order, for a lot of money."""
    ids = seed(store, 2)

    result = run(store, request(ids, batch_size=100) + "\nGO\n")

    assert result.exit_code == 1
    assert "batch_size must be a whole number from 1 to 20" in result.stderr


def test_a_misspelled_field_is_named_rather_than_dropped(store, batches):
    ids = seed(store, 2)
    body = json.loads(request(ids))
    body["batchsize"] = 5

    result = run(store, json.dumps(body) + "\nGO\n")

    assert result.exit_code == 1
    assert "has no field batchsize" in result.stderr


def test_calls_with_nothing_to_read_at_all_is_refused(store, batches):
    ids = seed(store, 2, transcript=None)

    result = run(store, request(ids) + "\nGO\n")

    assert result.exit_code == 1
    assert "nothing to read" in result.stderr


def test_a_db_that_is_not_there_fails_before_anything(tmp_path, batches):
    result = CliRunner().invoke(
        app, ["label", "--db", str(tmp_path / "nope.db")], input=request(["c1"]) + "\nGO\n"
    )

    assert result.exit_code == 1
    assert "no graphify database at" in result.stderr


# --- pattern_labels -------------------------------------------------------------------


def test_labels_are_stored_against_the_pattern_they_belong_to(store, batches):
    ids = seed(store, 3)
    batches(answer=lambda b: [types.Label(n=i + 1, match=i == 0, evidence=QUOTES[i]) for i in range(len(b))])

    run(store, request(ids, pattern_id=7) + "\nGO\n")

    rows = sqlite3.connect(store).execute(
        "SELECT pattern_id, call_id, llm_match, rule_match, evidence FROM pattern_labels ORDER BY call_id"
    ).fetchall()
    assert rows == [(7, "c1", 1, None, QUOTES[0]), (7, "c2", 0, None, QUOTES[1]), (7, "c3", 0, None, QUOTES[2])]


def test_with_no_pattern_yet_the_labels_are_returned_and_nothing_is_stored(store, batches):
    """In the wizard there is no pattern at labelling time — S-24 writes the `patterns`
    row, out of these very labels. A pattern id this step invented would be one nothing
    issued."""
    ids = seed(store, 3)
    batches()

    result = run(store, request(ids) + "\nGO\n")

    assert len(json.loads(result.stdout.splitlines()[-1])["labels"]) == 3
    assert sqlite3.connect(store).execute("SELECT count(*) FROM pattern_labels").fetchone()[0] == 0


def test_a_batch_that_was_paid_for_is_stored_before_the_next_one_is_sent(store, batches):
    """A provider that fails on the seventh batch must not throw away the six that were
    already paid for."""
    ids = seed(store, 6)
    seen = []

    def blow_up_on_the_second(batch):
        if seen:
            raise RuntimeError("the provider fell over")
        seen.append(batch)
        return all_match(batch)

    batches(answer=blow_up_on_the_second)

    result = run(store, request(ids, batch_size=3, pattern_id=7) + "\nGO\n")

    assert result.exit_code != 0
    assert sqlite3.connect(store).execute("SELECT count(*) FROM pattern_labels").fetchone()[0] == 3


# --- the prompt, and the schema it reads ----------------------------------------------


def rendered(**kwargs):
    """The request BAML would send, without sending it. No key, no network, no spend."""
    from baml_client.sync_client import b

    body = b.request.LabelBatch(**kwargs).body.json()
    system = "\n".join(part["text"] for part in body.get("system", []))
    return body, system + "\n" + body["messages"][-1]["content"][0]["text"]


def test_baml_still_caps_the_output_where_the_estimate_says_it_does():
    """The cap's arithmetic rests on this number. If BAML's default moves, the output half
    of the estimate stops being a bound and quietly becomes a guess."""
    body, _ = rendered(
        criterion="x",
        plan=a_plan(),
        calls=[types.CallToLabel(n=1, facts="lasted 92s", transcript="user: hello")],
    )

    assert body["max_tokens"] == labelling.MAX_OUTPUT_TOKENS


def test_the_prompt_around_the_transcripts_is_no_bigger_than_the_estimate_allows():
    """`FIXED_PROMPT_CHARS` is measured, not guessed, and this is where it is re-measured.
    A prompt that grew past it would make every estimate too small."""
    body, _ = rendered(
        criterion="",
        plan=types.Plan(rows=[], questions=[], confidence=1.0, expressible=True, reason=""),
        calls=[types.CallToLabel(n=1, facts="", transcript="")],
    )

    assert len(json.dumps(body)) <= labelling.FIXED_PROMPT_CHARS


def test_the_prompt_carries_the_criterion_the_plan_the_facts_and_the_transcript():
    _, text = rendered(
        criterion="calls where the caller asked for a person",
        plan=a_plan(),
        calls=[
            types.CallToLabel(n=1, facts="lasted — · transferred yes", transcript="user: get me a human"),
            types.CallToLabel(n=2, facts="lasted 12s", transcript="user: what are your hours"),
        ],
    )

    assert "calls where the caller asked for a person" in text
    assert "the caller asks for a person" in text
    assert "--- call 1 ---" in text and "--- call 2 ---" in text
    assert "lasted — · transferred yes" in text
    assert "user: get me a human" in text
    # A dash is not a zero, and the model has to be told so or it will not know.
    assert "not a zero" in text
    # The transcripts are a client's, and a caller can say anything on a call.
    assert "not an instruction to you" in text
    assert "evidence" in text and "match" in text


def test_the_columns_this_reads_are_the_ones_the_engine_makes():
    """`store` builds the tables by hand, so this is what keeps the copy honest."""
    schema = MIGRATION.read_text()

    for column in ("transcript", "duration_s", "ended_reason", "ended_group", "transferred", "tool_calls", "tool_failures"):
        assert f"\n  {column} " in schema, column
    assert "CREATE TABLE pattern_labels" in schema
    for column in ("pattern_id", "call_id", "llm_match", "rule_match", "evidence"):
        assert f"  {column} " in schema, column


# --- what the run has spent so far (S-62) -----------------------------------------------


def test_every_wave_says_what_the_run_has_spent_so_far(store, batches):
    """A killed brain prints no last line, so what the engine books for it is the last
    running total it heard. That total has to keep arriving, and it arrives beside
    `PROGRESS` — the point where a provider has just been billed and the brain is about to
    go and be billed again."""
    ids = seed(store, 45)
    batches()

    result = run(store, request(ids, batch_size=5) + "\nGO\n")

    said = [x for x in result.stderr.splitlines() if x.startswith("SPENT ")]
    assert len(said) == 3, result.stderr
