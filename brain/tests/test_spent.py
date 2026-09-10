"""What the brain says it was billed when it is on its way down.

Two halves. `spent` is arithmetic over whatever a collector holds, and it is tested against
collectors built here — mixed clients, missing usage, a name that does not price. Then one
test drives a real BAML call to prove the three facts the arithmetic rests on, none of which
are graphify's to decide: that a collector passed alongside another one is filled in, that
it sums across function logs, and that the log of a call which *raised* is in it.

That last test stands up an HTTP server on the loopback interface and points a runtime BAML
client at it. No provider is reached and nothing leaves the machine. It is here rather than
faked because the claim under test is BAML's behaviour, and a stub of BAML would only test
this file's opinion of it.
"""

from __future__ import annotations

import json
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Any

import pytest

from graphify_brain import cost


@pytest.fixture(autouse=True)
def _fresh_ledger() -> Any:
    """The ledger is scoped to a brain process, and a test process is not one.

    `test_booked.py` hands the call sites a stand-in for `baml_py.Collector`, so a run that
    touches it leaves the module global holding a fake. Reset around every test here so this
    file does not depend on what pytest ran before it.
    """
    cost._LEDGER = None
    yield
    cost._LEDGER = None


class FakeUsage:
    def __init__(self, tokens_in: int | None, tokens_out: int | None) -> None:
        self.input_tokens = tokens_in
        self.output_tokens = tokens_out


class FakeCall:
    def __init__(self, client: str, tokens_in: int | None, tokens_out: int | None) -> None:
        self.client_name = client
        self.usage = FakeUsage(tokens_in, tokens_out)


class FakeLog:
    def __init__(self, *calls: FakeCall) -> None:
        self.calls = list(calls)


class FakeCollector:
    def __init__(self, *logs: FakeLog) -> None:
        self.logs = list(logs)


def test_nothing_called_is_nothing_spent() -> None:
    assert cost.spent(FakeCollector()) == 0.0


def test_a_call_is_priced_at_its_own_client_and_not_the_process_average() -> None:
    """`daily` labels several patterns in one process and each carries its own model.

    Sonnet is $2/$10 per MTok and opus is $5/$25, so tokens summed first and priced once
    are wrong at both rates and wrong by more the further apart the two cards are.
    """
    mixed = FakeCollector(
        FakeLog(FakeCall("Sonnet", 1_000_000, 1_000_000)),
        FakeLog(FakeCall("Opus", 1_000_000, 1_000_000)),
    )
    assert cost.spent(mixed) == pytest.approx(2 + 10 + 5 + 25)

    # What summing first would have given, at either card. Neither is the answer above.
    assert cost.spent(mixed) != pytest.approx(cost.estimate(2_000_000, 2_000_000, "sonnet"))
    assert cost.spent(mixed) != pytest.approx(cost.estimate(2_000_000, 2_000_000, "opus"))


def test_every_client_the_brain_can_select_can_be_priced_on_the_way_down() -> None:
    """`CLIENTS` is the only way a model is chosen, so its values are the only names that
    can appear on a collected call. A rename that breaks the inverse would otherwise turn
    every failed job silent, and quietly."""
    for client in cost.CLIENTS.values():
        assert cost.spent(FakeCollector(FakeLog(FakeCall(client, 1_000, 100)))) > 0


def test_a_call_with_no_usage_adds_nothing() -> None:
    """The calls that come back without usage are the 5xx responses `retry_policy Backoff`
    retried, and a provider does not bill for those. This is not S-56's `or 0`: there is no
    charge to be missing, rather than a charge nobody reported."""
    retried = FakeLog(
        FakeCall("Sonnet", None, None),
        FakeCall("Sonnet", None, None),
        FakeCall("Sonnet", 1_000_000, 1_000_000),
    )
    assert cost.spent(FakeCollector(retried)) == pytest.approx(12)


def test_a_client_that_does_not_price_gives_up_on_the_whole_total() -> None:
    """A part of a total, booked as though it were all of it, is the defect this function
    exists to fix one layer further in. Nothing is better than nearly."""
    partial = FakeCollector(
        FakeLog(FakeCall("Sonnet", 1_000_000, 1_000_000)),
        FakeLog(FakeCall("Rumour", 1_000_000, 1_000_000)),
    )
    assert cost.spent(partial) is None


# --- what BAML actually does, on the loopback interface ---------------------------------

PLAN = json.dumps(
    {
        "rows": [{"if_": "the caller asked for a person", "then": "counts"}],
        "questions": [],
        "confidence": 0.9,
        "expressible": True,
        "reason": "ok",
    }
)


def _reply(text: str) -> bytes:
    return json.dumps(
        {
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-5",
            "content": [{"type": "text", "text": text}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 1_000_000, "output_tokens": 1_000_000},
        }
    ).encode()


@pytest.fixture
def endpoint() -> Any:
    """A stand-in for the anthropic endpoint. The third answer will not coerce."""
    served = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            served.append(self.path)
            body = _reply(PLAN if len(served) < 3 else "I am afraid I cannot help.")
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args: Any) -> None:
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{server.server_address[1]}"
    server.shutdown()


def test_a_collector_holds_the_call_that_raised(endpoint: str) -> None:
    """The fact the whole step rests on.

    Three calls through one collector, the third answered with something BAML cannot
    coerce. If the raised call were missing from the collector there would be nothing to
    book and no step here; if the collector did not sum, a labelling run would only ever
    know about its last batch.
    """
    from baml_py import ClientRegistry, Collector

    from baml_client.sync_client import b

    registry = ClientRegistry()
    registry.add_llm_client(
        "Sonnet",
        "anthropic",
        {"model": "claude-sonnet-5", "api_key": "unused", "max_tokens": 4096, "base_url": endpoint},
        "Backoff",
    )
    registry.set_primary("Sonnet")

    ledger = Collector("test")
    raised = False
    for _ in range(3):
        try:
            b.with_options(client_registry=registry, collector=[Collector(), ledger]).PlanPattern(
                criterion="asked for a human", system_prompt=None, dsl="dsl"
            )
        except Exception:
            raised = True

    assert raised, "the third answer coerced after all; this test proves nothing"
    assert len(ledger.logs) == 3, "a call that raised is not in the collector"
    # Three calls at a million tokens each way on sonnet's card, the third one billed for a
    # reply nobody could use. That is the money `cli.run` reports on the way out.
    assert cost.spent(ledger) == pytest.approx(3 * 12)


# --- the line the engine reads ----------------------------------------------------------


def _brain(*args: str, stdin: str = "") -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-c", "from graphify_brain.cli import run; run()", *args],
        input=stdin,
        capture_output=True,
        text=True,
    )


def test_a_clean_exit_prints_no_spend_line() -> None:
    """`version` is the cheapest command that succeeds. The extra line belongs to the
    failing half of the contract only — on the way out through the `Ok` branch the engine
    reads the last line as the job's result, and S-57 books what that result says."""
    done = _brain("version")
    assert done.returncode == 0
    assert "usd" not in done.stdout, done.stdout


def test_a_refused_request_says_it_spent_nothing() -> None:
    """A `ValueError` refusal happens with the model untouched, so zero is knowledge and
    not a shrug. The engine tells it apart from a brain that never said."""
    died = _brain("plan", stdin=json.dumps({"criterion": "x"}))
    assert died.returncode == 1
    assert json.loads(died.stdout.strip().splitlines()[-1]) == {"usd": 0.0}


def test_the_complaint_still_goes_to_stderr_and_only_the_number_to_stdout() -> None:
    """An exception message can carry the prompt and the model's raw reply, and stdout's
    last line is written to a column the browser reads. The traceback keeps its old home."""
    died = _brain("plan", stdin=json.dumps({"criterion": "x"}))
    assert died.stderr.strip(), "the complaint went missing"
    assert json.loads(died.stdout.strip().splitlines()[-1]) == {"usd": 0.0}
    assert "criterion" not in died.stdout, died.stdout
