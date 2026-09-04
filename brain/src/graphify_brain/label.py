"""Reading real transcripts, and paying for it.

This is where graphify first spends money at a scale a person would notice, so most of
this module is not about labelling at all. It is about the four rules that stand between
a criterion and a bill:

* **The price is shown first.** `ESTIMATE {usd}` goes to stdout before anything is read.
* **Nobody spends without saying so.** The run then waits for `GO` on stdin. Without it,
  no model is called and the output says `"stopped": "declined"`.
* **The cap is checked before every batch, not after.** A batch whose estimate would take
  the running total past `max_usd` is never sent. Checking afterwards would be finding out
  that the cap was passed, which is not a cap.
* **What is paid for is kept.** Labels are written to `pattern_labels` after each wave of
  batches, so a provider that fails on batch seven does not throw away batches one to six.

The estimate errs high on purpose, the way `graphify_brain.cost` says it must. Output is
priced at `MAX_OUTPUT_TOKENS`, the `max_tokens` BAML actually sends, so no batch can cost
more output than the estimate allowed for. Input is priced at three characters per token
against English speech that runs nearer four. A cap built on an under-estimate is not a
cap, and a person who approved a number should never be charged more than it.

This module judges nothing about the plan it is handed. A plan at 0.4 confidence labels
exactly as one at 1.0 — the gate that will not read calls below 0.95 lives in the wizard,
in front of the person paying, for the same reason it did in `plan.py`.
"""

from __future__ import annotations

import json
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from typing import Any, Iterator, Sequence, TextIO

from graphify_brain import cost
from graphify_brain.plan import envelope, required_text

#: Batches in flight at once. Three is the register's number: enough that a run of forty
#: calls is not three round trips end to end, few enough that a rate limit is unlikely and
#: that at most three batches are in doubt when something fails.
CONCURRENCY = 3

#: D-3: labelling loops over calls twenty at a time. The ceiling is here rather than only
#: in the default because a caller that asked for a hundred at once would be asking one
#: model call to hold a hundred transcripts and return a hundred labels in the right order.
MAX_BATCH = 20
DEFAULT_BATCH = 20

#: Characters per token, for the estimate. English prose runs nearer four, so this
#: over-counts by about a third — which is the direction an estimate guarding a cap has to
#: err. It is not a bound: a transcript full of punctuation or another script can tokenize
#: worse than this. Output is bounded, below; input is not.
CHARS_PER_TOKEN = 3

#: The prompt around the criterion, the plan and the transcripts — instructions and output
#: schema — measured from the rendered request and rounded up. `test_label.py` renders it
#: and fails if the real thing has grown past this, so the estimate cannot quietly start
#: under-counting when the prompt is edited.
FIXED_PROMPT_CHARS = 2_400

#: The `max_tokens` BAML sends with every call. This is what makes the output side of the
#: estimate a real bound rather than a guess: no batch can return more than this, so no
#: batch can cost more output than the cap check allowed for. `test_label.py` reads it back
#: out of a rendered request, so a change in BAML's default fails a test instead of quietly
#: turning the cap into an approximation.
MAX_OUTPUT_TOKENS = 4_096

#: The model nickname in the request, and the BAML client it selects. The keys are also
#: `cost.PRICES`'s keys, so a model that can be asked for is a model whose spend can be
#: counted — an unpriced model is refused rather than run against a total that never grows.
CLIENTS = {"opus": "Opus", "sonnet": "Sonnet", "gpt": "GPT"}

#: How many `?` placeholders to put in one SELECT. SQLite's default limit is 999 bound
#: variables; five hundred leaves room and costs one extra round trip per five hundred
#: calls, which is nothing next to reading them.
SQL_VARS = 500

#: A value nobody recorded. The spec's rule, verbatim: a missing value is never rendered
#: as 0. It matters more here than anywhere — a model told a call lasted 0 seconds will
#: reason about a call that never connected.
DASH = "—"


@dataclass(frozen=True)
class Call:
    """One call, ready to be shown to a model: its id, what the system recorded, and what
    was said."""

    id: str
    facts: str
    transcript: str


@dataclass(frozen=True)
class Job:
    """A checked request. Nothing here is still in doubt, and no money has been spent."""

    criterion: str
    plan: Any
    """The `types.Plan` from S-22, validated. Passed to the model whole."""
    plan_text: str
    """The same plan as JSON, kept only so the estimate can count its characters without
    serialising it once per batch."""
    model: str
    batch_size: int
    max_usd: float
    pattern_id: int | None
    calls: list[Call]
    no_transcript: list[str]


def client() -> Any:
    """The generated BAML client — the same seam as `plan.client`.

    Nothing else in this module reaches a provider, so a test that replaces this has
    replaced every way out.
    """
    from baml_client.sync_client import b

    return b


def run(stdin: TextIO, stdout: TextIO, stderr: TextIO, conn: Any, yes: bool) -> None:
    """The whole command: read the request, show the price, wait for the go, label.

    The request is **one line** of JSON, not the whole of stdin, because there is a second
    message coming on the same pipe. `plan` and `clarify` read to EOF; this one cannot,
    or the `GO` would already have been swallowed by the time it was asked for.
    """
    job = prepare(_request(stdin), conn)
    print(f"ESTIMATE {estimate(job):.4f}", file=stdout, flush=True)

    if not yes and not _go(stdin):
        # A person declining to spend is not a failure, so this exits 0. `"stopped"` is
        # what tells the engine that the calls went unread on purpose.
        print("no GO on stdin; nothing was read and nothing was spent", file=stderr)
        print(json.dumps(_result(job, [], [], [c.id for c in job.calls], 0.0, 0, "declined")), file=stdout)
        return

    print(json.dumps(_label(job, conn, stderr)), file=stdout)


def label_calls(payload: Any, conn: Any, stdout: TextIO, stderr: TextIO) -> dict[str, Any]:
    """Show the price and label, with no `GO` between the two.

    `run` is the command a person is at the other end of: it reads a request off stdin and
    waits for somebody to approve the price before a call is read. `daily` has nobody to
    ask at six in the morning, and D-8 puts two hard caps in place of the click. So it
    needs the same two steps — the price said out loud, then the labelling — without the
    conversation in the middle.

    The cap still does all the work it does in `run`: `max_usd` is checked before every
    batch, by the same function, so the price printed here is a ceiling and not a promise.
    """
    job = prepare(payload, conn)
    print(f"ESTIMATE {estimate(job):.4f}", file=stdout, flush=True)
    return _label(job, conn, stderr)


def prepare(payload: Any, conn: Any) -> Job:
    """Check the request and read the calls it names. Raises `ValueError` for anything
    wrong with either, with no model touched."""
    envelope(
        payload,
        "label",
        required={"criterion", "plan", "call_ids", "model", "max_usd"},
        optional={"batch_size", "pattern_id"},
    )
    # `baml_client` is generated, so it is imported inside the function for the reason
    # given in `plan.client`.
    from baml_client import types

    criterion = required_text(payload, "criterion")
    plan = types.Plan.model_validate(payload["plan"])
    model = _model(payload["model"])
    batch_size = _batch_size(payload.get("batch_size", DEFAULT_BATCH))
    max_usd = _max_usd(payload["max_usd"])
    pattern_id = _pattern_id(payload.get("pattern_id"))
    ids = _call_ids(payload["call_ids"])

    rows = _read_calls(conn, ids)
    if missing := [i for i in ids if i not in rows]:
        # Refused, not skipped. S-24 divides by the number of labels to get an agreement
        # figure; labelling forty-four of the forty-five calls somebody asked about would
        # make that figure quietly wrong about which calls it describes.
        raise ValueError(
            f"label: {len(missing)} of these call ids are not in the database, "
            f"the first is {missing[0]}"
        )

    called, failed = _read_tools(conn, ids)
    calls, no_transcript = [], []
    for i in ids:
        row = rows[i]
        text = (row["transcript"] or "").strip()
        if not text:
            # Not sent, and not labelled either. There is nothing for the model to read,
            # and paying it to say so would buy a label that means "we do not know"
            # dressed up as one that means "no".
            no_transcript.append(i)
            continue
        calls.append(Call(i, _facts(row, called[i], failed[i]), text))

    if not calls:
        raise ValueError("label: none of these calls has a transcript, so there is nothing to read")

    return Job(criterion, plan, plan.model_dump_json(), model, batch_size, max_usd, pattern_id, calls, no_transcript)


def batches(job: Job) -> list[list[Call]]:
    """The calls cut into the batches that will be sent.

    One place, so the number the estimate priced and the number the loop sends can never
    come to disagree.
    """
    return list(_chunks(job.calls, job.batch_size))


def estimate(job: Job) -> float:
    """USD for the whole job, at the ceiling. What `ESTIMATE` prints."""
    return sum(batch_usd(job, batch) for batch in batches(job))


def batch_usd(job: Job, batch: Sequence[Call]) -> float:
    """USD for one batch, at the ceiling. What the cap is checked against."""
    chars = (
        FIXED_PROMPT_CHARS
        + len(job.criterion)
        + len(job.plan_text)
        + sum(len(c.facts) + len(c.transcript) for c in batch)
    )
    return cost.estimate(chars // CHARS_PER_TOKEN, MAX_OUTPUT_TOKENS, job.model)


def call_batch(job: Job, batch: Sequence[Call]) -> tuple[list[Any], float]:
    """Label one batch. The only function here that calls a model, and the only one that
    finds out what a call actually cost.

    The calls are numbered from one within the batch and the model answers by number. It
    never sees a call id: twenty UUIDs copied back is tokens paid for nothing, and one
    transposed character would attach a label to the wrong call in a way no test catches.
    """
    from baml_py import Collector

    from baml_client import types

    numbered = [types.CallToLabel(n=i + 1, facts=c.facts, transcript=c.transcript) for i, c in enumerate(batch)]
    collector = Collector()
    got = client().with_options(client=CLIENTS[job.model], collector=collector).LabelBatch(
        criterion=job.criterion, plan=job.plan, calls=numbered
    )
    usage = collector.last.usage
    return list(got), cost.estimate(usage.input_tokens or 0, usage.output_tokens or 0, job.model)


def _label(job: Job, conn: Any, stderr: TextIO) -> dict[str, Any]:
    """The loop: waves of `CONCURRENCY` batches, priced before each one is sent.

    In the order the calls were given, and stopping at the first batch the cap cannot
    afford rather than skipping it for a smaller one further down. The calls are the
    analyst's sample; a run that labelled a scattered subset of it because those were the
    cheap ones would produce an agreement figure about a sample nobody chose.
    """
    sending = batches(job)
    order = {call.id: i for i, call in enumerate(job.calls)}
    labels: list[dict[str, Any]] = []
    no_label: list[str] = []
    unreached: list[str] = []
    spent = 0.0
    done = 0
    stopped: str | None = None

    with ThreadPoolExecutor(max_workers=CONCURRENCY) as pool:
        for wave in _chunks(sending, CONCURRENCY):
            fits = _affordable(job, wave, spent)
            if len(fits) < len(wave):
                stopped = "cap"

            fresh: list[dict[str, Any]] = []
            failure: Exception | None = None
            # `as_completed` and not `map`, so that one batch falling over does not throw
            # away the answers of the two beside it that arrived and were charged for.
            # The future remembers which batch it was, which is what keeps a label
            # attached to the right call when three of them come back out of order.
            sent = {pool.submit(call_batch, job, batch): batch for batch in fits}
            for future in as_completed(sent):
                try:
                    got, usd = future.result()
                except Exception as e:  # re-raised below, once this wave has been stored
                    failure = failure or e
                    continue
                spent += usd
                labelled, unlabelled = _attach(sent[future], got)
                fresh.extend(labelled)
                no_label.extend(unlabelled)

            # Back into the order the calls were asked about in. They came back in
            # whatever order three providers replied, and a list that reorders itself run
            # to run is one nobody can diff.
            fresh.sort(key=lambda x: order[x["call_id"]])
            no_label.sort(key=lambda i: order[i])

            # Written before the next wave is sent, and written even when this one failed.
            # A provider that falls over on the seventh batch must not throw away the six
            # that were already paid for.
            _write(conn, job.pattern_id, fresh)
            labels.extend(fresh)
            done += len(fits)
            print(f"PROGRESS {done}/{len(sending)}", file=stderr, flush=True)

            if failure is not None:
                raise failure
            if stopped:
                break

    if stopped:
        # Every batch from the first one that did not fit onwards, including the whole
        # waves after it that were never even priced.
        unreached = [c.id for b in sending[done:] for c in b]
        # S-28 reads this line out of the job log to know a run was cut short rather than
        # finished.
        print(
            f"cap reached after {done} of {len(sending)} batches; "
            f"${spent:.4f} spent of ${job.max_usd:.4f}",
            file=stderr,
            flush=True,
        )

    return _result(job, labels, no_label, unreached, spent, done, stopped)


def _affordable(job: Job, wave: Sequence[Sequence[Call]], spent: float) -> list[Sequence[Call]]:
    """The batches from this wave that fit under the cap.

    Priced at the estimate and checked *before* the wave is sent, because there is no way
    to know what a call cost until it has been made and paid for. The whole wave is
    reserved up front — the three run at once, so the cap has to hold for all three
    together and not for each in turn.
    """
    fit: list[Sequence[Call]] = []
    running = spent
    for batch in wave:
        due = batch_usd(job, batch)
        if running + due > job.max_usd:
            break
        running += due
        fit.append(batch)
    return fit


def _attach(batch: Sequence[Call], got: Sequence[Any]) -> tuple[list[dict[str, Any]], list[str]]:
    """Model's `n` back to the call it was about, plus the calls it did not answer for.

    `pop` rather than a lookup, so a number returned twice is used once: the second label
    for a call finds nothing left and is dropped, and the call is not counted as unlabelled
    either. A number that was not in the batch is dropped for the same reason — it is about
    a call this batch did not contain.
    """
    by_n = {i + 1: call for i, call in enumerate(batch)}
    labelled = []
    for label in got:
        call = by_n.pop(label.n, None)
        if call is None:
            continue
        labelled.append({"call_id": call.id, "match": label.match, "evidence": label.evidence})
    return labelled, [c.id for c in by_n.values()]


def _write(conn: Any, pattern_id: int | None, labels: Sequence[dict[str, Any]]) -> None:
    """Store this wave's labels, when there is a pattern to store them against.

    `pattern_id` is optional because in the wizard there is not one yet: S-24 writes the
    `patterns` row, after these labels have been synthesised into a rule. So the first run
    of a pattern's life returns its labels and stores nothing, and S-28's daily runs — which
    label against a pattern that exists — pass an id and store them. `rule_match` is left
    NULL; it is S-24's `rule-check` that fills it in.
    """
    if pattern_id is None or not labels:
        return
    conn.executemany(
        "INSERT INTO pattern_labels (pattern_id, call_id, llm_match, rule_match, evidence) "
        "VALUES (?, ?, ?, NULL, ?)",
        [(pattern_id, x["call_id"], int(x["match"]), x["evidence"]) for x in labels],
    )
    conn.commit()


def _result(
    job: Job,
    labels: list[dict[str, Any]],
    no_label: list[str],
    unreached: list[str],
    spent: float,
    batches_done: int,
    stopped: str | None,
) -> dict[str, Any]:
    """Every call that was asked about appears in exactly one of the four lists, and each
    list has exactly one cause: it was labelled, it had nothing to read, the model did not
    answer for it, or the run stopped before it."""
    return {
        "labels": labels,
        "no_transcript": job.no_transcript,
        "no_label": no_label,
        "not_reached": unreached,
        "usd": round(spent, 6),
        "batches": batches_done,
        "model": job.model,
        "stopped": stopped,
    }


# --- reading the calls ----------------------------------------------------------------


def _read_calls(conn: Any, ids: Sequence[str]) -> dict[str, Any]:
    """The columns a labeller needs: what was said, and what the system recorded."""
    rows = {}
    for chunk in _chunks(ids, SQL_VARS):
        found = conn.execute(
            "SELECT id, transcript, duration_s, ended_reason, ended_group, transferred, "
            f"tool_calls, tool_failures FROM calls WHERE id IN ({_marks(chunk)})",
            list(chunk),
        )
        for row in found:
            rows[row["id"]] = row
    return rows


def _read_tools(conn: Any, ids: Sequence[str]) -> tuple[dict[str, list[str]], dict[str, list[str]]]:
    """Which tools ran on each call, and which of them failed, by name."""
    called: dict[str, list[str]] = defaultdict(list)
    failed: dict[str, list[str]] = defaultdict(list)
    for chunk in _chunks(ids, SQL_VARS):
        found = conn.execute(
            f"SELECT call_id, name, failed FROM tool_calls WHERE call_id IN ({_marks(chunk)})",
            list(chunk),
        )
        for row in found:
            called[row["call_id"]].append(row["name"])
            if row["failed"]:
                failed[row["call_id"]].append(row["name"])
    return called, failed


def _facts(row: Any, called: Sequence[str], failed: Sequence[str]) -> str:
    """The line above each transcript: what the system recorded about the call.

    Here because a plan row can be about something nobody says out loud. "Calls that ended
    in an error", "calls where the booking tool failed", "calls under thirty seconds" are
    all conditions the rule DSL can check and no transcript can show, and a labeller that
    could not see them would be guessing at exactly the calls S-24 measures its rule
    against.

    Every unknown is a dash, and the prompt says a dash is not a zero. `tool_calls` is the
    count the sync recorded, so it — not the emptiness of the list — is what separates "no
    tool ran" from "nobody recorded whether one did".
    """
    return " · ".join(
        [
            f"lasted {_duration(row['duration_s'])}",
            f"ended {row['ended_reason'] or DASH} ({row['ended_group'] or DASH})",
            f"transferred {_yes_no(row['transferred'])}",
            f"tools run {_tools(row['tool_calls'], called)}",
            f"tools failed {_tools(row['tool_failures'], failed)}",
        ]
    )


def _duration(seconds: Any) -> str:
    return DASH if seconds is None else f"{float(seconds):.0f}s"


def _yes_no(flag: Any) -> str:
    return DASH if flag is None else ("yes" if flag else "no")


def _tools(counted: Any, names: Sequence[str]) -> str:
    if counted is None:
        return DASH
    return ", ".join(names) if names else "none"


# --- checking the request -------------------------------------------------------------


def _model(value: Any) -> str:
    known = ", ".join(sorted(CLIENTS))
    if not isinstance(value, str) or value.strip().lower() not in CLIENTS:
        raise ValueError(f"label: model must be one of {known}, not {value!r}")
    return value.strip().lower()


def _batch_size(value: Any) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or not 1 <= value <= MAX_BATCH:
        raise ValueError(f"label: batch_size must be a whole number from 1 to {MAX_BATCH}, not {value!r}")
    return value


def _max_usd(value: Any) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or value <= 0:
        # No default. A cap that a caller can leave out is a cap that gets left out, and
        # "must not exceed max_usd" means nothing when there is no max_usd.
        raise ValueError(f"label: max_usd must be a positive number, not {value!r}")
    return float(value)


def _pattern_id(value: Any) -> int | None:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"label: pattern_id must be a whole number, not {value!r}")
    return value


def _call_ids(value: Any) -> list[str]:
    if not isinstance(value, list) or not value:
        raise ValueError("label: call_ids is empty, so there is nothing to read")
    if any(not isinstance(i, str) or not i.strip() for i in value):
        raise ValueError("label: every call id must be a non-empty string")
    ids = [i.strip() for i in value]
    if len(set(ids)) != len(ids):
        # A repeated id would be read twice and paid for twice, and would then land in
        # `pattern_labels` twice with two answers to the same question.
        raise ValueError("label: call_ids repeats an id")
    return ids


# --- stdin ---------------------------------------------------------------------------


def _request(stdin: TextIO) -> Any:
    line = stdin.readline()
    try:
        return json.loads(line)
    except json.JSONDecodeError as e:
        raise ValueError(f"stdin is not JSON: {e}") from e


def _go(stdin: TextIO) -> bool:
    """`GO`, exactly, on its own line. EOF, silence, or anything else is a no."""
    return stdin.readline().strip() == "GO"


# --- lists ---------------------------------------------------------------------------


def _chunks(items: Sequence[Any], size: int) -> Iterator[list[Any]]:
    for start in range(0, len(items), size):
        yield list(items[start : start + size])


def _marks(items: Sequence[Any]) -> str:
    return ",".join("?" * len(items))
