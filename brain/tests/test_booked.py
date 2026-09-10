"""What a call costs when the provider does not say.

Four functions in this package turn a provider's answer into money — `plan.charged`,
`label.call_batch`, `ask.ask`, `synth._spent` — and every one of them used to read
`usage.input_tokens or 0`. `Usage.input_tokens` is `Optional[int]` in BAML's own type
signature, so `or 0` is this brain's answer to a provider that reports no usage: the call
was free.

Zero is not a spare value. It is the correct booking for a `daily` run in free mode, for a
run the cap stopped before it sent anything, and for a pattern recounted by rule alone —
and `engine/src/db.rs` writes no `spend` row for a job that cost zero. So the number that
means "this cost nothing" and the number that means "nobody knows what this cost" were the
same number, and the second one is a model call the day's ledger has no record of. That is
the second Must-never, because `sync.rs` computes the day's remaining budget by subtracting
that ledger from the cap.

The run cap goes first and it goes closer to home: `label` accumulates what `call_batch`
returns and reserves the next wave against it, so a `spent` stuck at zero makes every wave
look like the first one. `test_a_silent_provider_does_not_buy_a_bigger_cap` is that run.

Nothing here calls a model. The collector is the seam every one of these functions was
already written around.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

import pytest

from graphify_brain import ask as asking
from graphify_brain import cost
from graphify_brain import label as labelling
from graphify_brain import plan as planning
from graphify_brain import synth as synthesis
from baml_client import types
from test_label import batches, priced, request, run, seed, store  # noqa: F401

SRC = Path(__file__).resolve().parents[1] / "src" / "graphify_brain"

#: A ceiling no real call could reach, so a cell that books it is unmistakable.
CEILING = 7.25


class FakeUsage:
    def __init__(self, tokens_in: int | None, tokens_out: int | None):
        self.input_tokens = tokens_in
        self.output_tokens = tokens_out


class FakeCollector:
    def __init__(self, usage: FakeUsage):
        self.last = type("Last", (), {"usage": usage})()


#: The seams that take a collector, called the way their modules call them.
#:
#: `label.call_batch` and `ask.ask` take a job and a client instead, so they are not in the
#: table — they are in the two runs below, where a whole `label` job and a whole `ask` go
#: through a provider that prices nothing. What makes the table complete anyway is the
#: harvest: `cost.booked` is the only function in the package that reads a token count off
#: a provider, so a path that books at all books through this arithmetic.
BOOKERS: dict[str, Any] = {
    "plan.charged": lambda usage, model: planning.charged(FakeCollector(usage), model, CEILING),
    "synth._spent": lambda usage, model: synthesis._spent(FakeCollector(usage), model, CEILING),
    "cost.booked": lambda usage, model: cost.booked(usage, model, CEILING),
}

#: What the provider said, and what must be booked for it. `None` on either count is the
#: provider declining to say, whatever the other one holds.
SAID: dict[str, tuple[FakeUsage, float]] = {
    "both counts": (FakeUsage(1_000_000, 0), 2.00),
    "no input count": (FakeUsage(None, 1_000), CEILING),
    "no output count": (FakeUsage(1_000_000, None), CEILING),
    "neither count": (FakeUsage(None, None), CEILING),
}


# --- the table -------------------------------------------------------------------------


@pytest.mark.parametrize("said", sorted(SAID))
@pytest.mark.parametrize("booker", sorted(BOOKERS))
def test_a_call_the_provider_did_not_price_is_booked_at_its_ceiling(booker: str, said: str):
    """The acceptance, once per seam per answer.

    `sonnet` is $2.00 per million in, so the "both counts" row is the provider's own
    arithmetic and every other row is the ceiling — never zero, and never a mixture of a
    real count with a ceiling, which would be a third number that is neither what was
    billed nor what was quoted.
    """
    usage, want = SAID[said]

    assert BOOKERS[booker](usage, "sonnet") == pytest.approx(want)


@pytest.mark.parametrize("said", sorted(SAID))
def test_nothing_the_provider_can_say_books_a_call_at_nothing(said: str):
    """The rule under the table, said once without the arithmetic. A ceiling of zero is
    the only zero, and no ceiling here is zero."""
    usage, _ = SAID[said]

    assert cost.booked(usage, "gpt", CEILING) > 0.0


# --- the money one ---------------------------------------------------------------------


class Silent:
    """A BAML client that answers and reports no usage, and the collector that goes with it.

    `call_batch` and `ask` build their own `Collector` from `baml_py`, so that is what is
    replaced: everything else about those functions — the ceiling they pass, the arithmetic
    `cost.booked` does with it — is the real thing under test.
    """

    def __init__(self, answer):
        self.answer = answer
        self.sent = 0

    # -- stands in for `baml_py.Collector`
    def collector(self):
        outer = self

        class Collector:
            def __init__(self):
                self.last = type("Last", (), {"usage": FakeUsage(None, None)})()

        return Collector

    # -- stands in for `label.client` / `ask.client`
    def __call__(self):
        return self

    def with_options(self, client, collector):
        return self

    def LabelBatch(self, criterion, plan, calls):
        self.sent += 1
        return self.answer(calls)

    def AskAnalysis(self, **_):
        self.sent += 1
        return "an answer"


@pytest.fixture
def silent(monkeypatch):
    """A provider that answers every call and prices none of them."""

    def install(module, answer=None):
        fake = Silent(answer or (lambda calls: [
            types.Label(n=c.n, match=True, evidence="user: I want to talk to a person please")
            for c in calls
        ]))
        monkeypatch.setattr(module, "client", fake)
        monkeypatch.setattr("baml_py.Collector", fake.collector())
        return fake

    return install


def test_a_silent_provider_does_not_buy_a_bigger_cap(store, silent):
    """The run that measured this step, asserted.

    Thirty calls, three to a batch, and a cap set to exactly one wave. `label` reserves
    each wave against what it has booked so far, so a provider that priced nothing left
    `spent` at zero and every wave was checked as though it were the first: ten batches
    sent against a cap that fits three, reported as `$0.000000` and `stopped: null` — the
    brain saying it finished within budget.
    """
    ids = seed(store, 30)
    _, per_batch = priced(store, ids, batch_size=3)
    cap = per_batch[0] * 3

    fake = silent(labelling)
    result = run(store, request(ids, batch_size=3, max_usd=cap) + "\nGO\n")
    out = json.loads(result.stdout.splitlines()[-1])

    assert fake.sent == 3
    assert out["stopped"] == "cap"
    assert out["usd"] == pytest.approx(cap, rel=1e-6)


# --- the harvest -----------------------------------------------------------------------


def test_only_cost_py_reads_a_token_count_off_a_provider():
    """A fifth spend path cannot bring the `or 0` back.

    Read over the files a person edits, and the failure names the file that did it. This
    is the guard that outlives the defect: the rule lives in one function, and a module
    that reaches past it to `usage` is reaching past the rule.
    """
    reaching = sorted(
        f.name
        for f in SRC.glob("*.py")
        if f.name != "cost.py" and re.search(r"\.(input|output)_tokens\b", f.read_text())
    )

    assert reaching == []


# --- the ceiling is the site's own -------------------------------------------------------


def test_what_a_silent_batch_books_is_what_that_batch_quoted(store, silent):
    """Not a new number invented at the booking.

    `label` books a silent batch at `batch_usd`, which is the figure `_affordable` already
    reserved it against and the figure the analyst was shown. If the two ever came apart, a
    run could book more than it was allowed to spend, or reserve more than it books.
    """
    ids = seed(store, 6)
    _, per_batch = priced(store, ids, batch_size=3)

    fake = silent(labelling)
    result = run(store, request(ids, batch_size=3, max_usd=100.0) + "\nGO\n")
    out = json.loads(result.stdout.splitlines()[-1])

    assert fake.sent == 2
    assert out["usd"] == pytest.approx(sum(per_batch), rel=1e-6)


def test_a_silent_ask_books_its_own_estimate(silent):
    """`ask` sends one call, so its ceiling is the whole job's estimate."""
    job = asking.Job(
        question="who asked for a person",
        stats="{}",
        model="sonnet",
        max_usd=100.0,
        calls=[asking.Call("c1", "duration 92s", "user: I want to talk to a person please")],
        no_transcript=[],
    )
    silent(asking)

    _, usd = asking.ask(job)

    assert usd == pytest.approx(asking.estimate(job))
