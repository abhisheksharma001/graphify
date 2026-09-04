"""One free-form question about a selection of calls, priced before it is asked.

This is the only thing the brain does whose answer nothing downstream can check. A plan
becomes a table somebody reads back, a label becomes a boolean, a rule becomes JSON the
engine runs — an answer here is prose, and the person who reads it has only the prompt's
own honesty standing behind it. So the interesting parts of this module are the two that
keep it inside its bounds.

**The context is capped before the price is quoted, and again before it is sent.** The
engine picks which calls go in: it builds the statistics, takes the shortest transcripts
of the selection until `MAX_CONTEXT_TOKENS` is reached, and quotes that. This module reads
the same calls back out of the database and prices what it actually holds, then refuses if
that is over the cap or over the `max_usd` the engine was given. Two counts of one thing
sounds like duplication and is not: the engine's is what the person approved, and this one
is what is about to be sent, and a run where those two differ is a run that has to stop.

**No `GO`.** The click already happened. The engine quotes this question over a route that
starts no job and holds no process — cancelling at the price costs a round trip and leaves
nothing behind — and only a confirmed price gets as far as this program. A second approval
on stdin would be asking the same person the same question twice.

`ESTIMATE` is still printed, because the engine reads a job's quote back out of its log and
the browser shows the quote beside what the answer actually cost.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, TextIO

from graphify_brain import cost

# Half a dozen names come from `label` rather than being written again here. The two that
# matter are `CLIENTS` and `_model`: the models a question may be asked of are exactly the
# models a label may be bought from, so there is one list, one price table and one set of
# keys rather than two that can come to disagree.
from graphify_brain.label import (
    CHARS_PER_TOKEN,
    CLIENTS,
    MAX_OUTPUT_TOKENS,
    Call,
    _facts,
    _model,
    _max_usd,
    _read_calls,
    _read_tools,
)
from graphify_brain.plan import envelope, required_text

#: The register's ceiling on what one question may send. Counted over the statistics, the
#: question and every transcript together, at `CHARS_PER_TOKEN` — the same over-counting
#: `label` does, for the same reason: a cap built on an under-estimate is not a cap.
MAX_CONTEXT_TOKENS = 60_000

#: How many transcripts may go in, whatever the token count says. The engine picks them,
#: shortest first; this is the same number, checked again, because the list arrives over a
#: pipe and a request is not a promise.
MAX_CALLS = 20

#: What the prompt around the question, the statistics and the transcripts costs — the
#: instructions and the output schema, measured off a rendered request and rounded up.
#: `test_ask.py` re-measures it, so a prompt that grows past it fails a test instead of
#: quietly making every estimate too small.
FIXED_PROMPT_CHARS = 2_600


@dataclass(frozen=True)
class Job:
    """A checked request with its calls read. No money has been spent."""

    question: str
    stats: str
    model: str
    max_usd: float
    calls: list[Call]
    no_transcript: list[str]


def client() -> Any:
    """The generated BAML client — the same seam as `plan.client` and `label.client`, and
    the only way out of this module."""
    from baml_client.sync_client import b

    return b


def run(stdin: TextIO, stdout: TextIO, stderr: TextIO, conn: Any) -> None:
    """The whole command: read the request, say the price, answer.

    One line of stdin, like `label` — not because anything else is coming on the pipe, but
    because the engine writes one line and the shape of a brain request is that line.
    """
    job = prepare(_request(stdin), conn)
    quoted = estimate(job)
    print(f"ESTIMATE {quoted:.4f}", file=stdout, flush=True)

    if quoted > job.max_usd:
        # Not an error and not a failure: the engine quoted this question, somebody
        # approved that figure, and what arrived here prices higher than it. Something
        # moved between the quote and the go — a sync landed, a call was purged. Exiting 0
        # with nothing spent is what lets the browser say "ask again" instead of showing a
        # traceback for a thing nobody did wrong.
        print(
            f"the question now prices at ${quoted:.4f}, over the ${job.max_usd:.4f} that "
            "was approved; nothing was sent",
            file=stderr,
        )
        print(json.dumps(_result(job, None, 0.0, "cap")), file=stdout)
        return

    answer, usd = ask(job)
    print(json.dumps(_result(job, answer, usd, None)), file=stdout)


def prepare(payload: Any, conn: Any) -> Job:
    """Check the request and read the calls it names. Raises `ValueError` for anything
    wrong with either, with no model touched."""
    envelope(
        payload,
        "ask",
        required={"question", "stats", "model", "call_ids", "max_usd"},
        optional=set(),
    )
    question = required_text(payload, "question")
    stats = _stats(payload["stats"])
    model = _model(payload["model"], "ask")
    max_usd = _max_usd(payload["max_usd"], "ask")
    ids = _call_ids(payload["call_ids"])

    rows = _read_calls(conn, ids)
    if missing := [i for i in ids if i not in rows]:
        # Refused rather than skipped, as in `label`: the engine priced this question
        # against a named set of calls, and answering it over a smaller one would be an
        # answer about a selection nobody chose.
        raise ValueError(
            f"ask: {len(missing)} of these call ids are not in the database, "
            f"the first is {missing[0]}"
        )

    called, failed = _read_tools(conn, ids)
    calls, no_transcript = [], []
    for i in ids:
        row = rows[i]
        text = (row["transcript"] or "").strip()
        if not text:
            # The engine only picks calls that have one, so this is a call purged or
            # emptied between the quote and now. Named in the result rather than dropped
            # silently: the answer is about fewer calls than the price said.
            no_transcript.append(i)
            continue
        calls.append(Call(i, _facts(row, called[i], failed[i]), text))

    job = Job(question, stats, model, max_usd, calls, no_transcript)
    if tokens(job) > MAX_CONTEXT_TOKENS:
        # The engine's cap, checked again on this side. It is the one bound in this step
        # that is not about money, so no spend limit would catch it being passed.
        raise ValueError(
            f"ask: this question carries {tokens(job)} tokens of context, over the "
            f"{MAX_CONTEXT_TOKENS} a question may send"
        )
    return job


def tokens(job: Job) -> int:
    """Input tokens for the whole request, at the over-counting rate `label` uses."""
    chars = (
        FIXED_PROMPT_CHARS
        + len(job.question)
        + len(job.stats)
        + sum(len(c.facts) + len(c.transcript) for c in job.calls)
    )
    return chars // CHARS_PER_TOKEN


def estimate(job: Job) -> float:
    """USD at the ceiling. Output is priced at `MAX_OUTPUT_TOKENS`, the `max_tokens` BAML
    sends, so the answer cannot cost more output than the quote allowed for."""
    return cost.estimate(tokens(job), MAX_OUTPUT_TOKENS, job.model)


def ask(job: Job) -> tuple[str, float]:
    """Ask the question. The only function here that calls a model, and the only one that
    finds out what it actually cost.

    The calls are numbered from one, and the model never sees a call id — the same reason
    `label` gives: ids are UUIDs, and the numbering is what the prompt refers to them by.
    """
    from baml_py import Collector

    from baml_client import types

    numbered = [
        types.CallToLabel(n=i + 1, facts=c.facts, transcript=c.transcript)
        for i, c in enumerate(job.calls)
    ]
    collector = Collector()
    answer = (
        client()
        .with_options(client=CLIENTS[job.model], collector=collector)
        .AskAnalysis(question=job.question, stats=job.stats, calls=numbered)
    )
    usage = collector.last.usage
    return answer, cost.estimate(usage.input_tokens or 0, usage.output_tokens or 0, job.model)


def _result(job: Job, answer: str | None, usd: float, stopped: str | None) -> dict[str, Any]:
    return {
        "answer": answer,
        "calls": [c.id for c in job.calls],
        "no_transcript": job.no_transcript,
        "usd": round(usd, 6),
        "model": job.model,
        "stopped": stopped,
    }


def _stats(value: Any) -> str:
    """The engine's `/api/stats` answer, as the engine serialised it.

    A string and not an object: it is shown to a model verbatim, and re-serialising a
    parsed copy would mean the characters this module priced are not the characters that
    were sent.
    """
    if not isinstance(value, str) or not value.strip():
        raise ValueError("ask: stats must be the selection's statistics as a JSON string")
    return value.strip()


def _call_ids(value: Any) -> list[str]:
    if not isinstance(value, list):
        raise ValueError("ask: call_ids must be a list")
    if any(not isinstance(i, str) or not i.strip() for i in value):
        raise ValueError("ask: every call id must be a non-empty string")
    ids = [i.strip() for i in value]
    if len(set(ids)) != len(ids):
        raise ValueError("ask: call_ids repeats an id")
    if len(ids) > MAX_CALLS:
        raise ValueError(f"ask: a question may carry at most {MAX_CALLS} transcripts, not {len(ids)}")
    # An empty list is allowed. A question about the shape of a selection — how many calls
    # ended in an error, when they cluster — is answered out of the statistics, and a
    # window whose calls all lost their transcripts to retention still has those.
    return ids


def _request(stdin: TextIO) -> Any:
    line = stdin.readline()
    try:
        return json.loads(line)
    except json.JSONDecodeError as e:
        raise ValueError(f"stdin is not JSON: {e}") from e
