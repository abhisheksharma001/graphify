"""The daily run: reading new calls for the patterns that have a model in the loop.

`free` patterns are decided by their rule and cost nothing, so nothing here touches one.
The other two modes of D-8 are what this command is:

* **hybrid** — the rule is a prefilter. The calls it matched and nobody has read yet go to
  the model, which confirms or overrules each one.
* **full** — the model reads every new call in the org, whatever the rule thinks.

There is nobody at the other end of this at six in the morning, so the click that guards
every other model call is replaced by two caps, and both are checked before a batch is
sent rather than after it:

* **The pattern's own `daily_cap_usd`**, which bounds one pattern's reading in one run.
* **What is left of the org's day**, which the engine works out from the `spend` table and
  passes in as `max_usd`. It bounds every pattern in this run together, so a first pattern
  that eats the budget leaves the second unread rather than doubling the bill.

Two rules shape the rest of it.

**The spend is reported, always.** The engine books what this run cost from the last line
it prints, so a pattern that fails takes its own labels down and nothing else: the failure
is recorded against that pattern, the run carries on, and the total that reaches `spend`
is the total that was actually paid. A traceback out of here would be money spent and no
line to book it from.

**The newest calls are read first.** A capped run reads part of what it was given, so the
part it reads should be the part somebody is about to look at.
"""

from __future__ import annotations

import json
import traceback
from dataclasses import dataclass
from typing import Any, Sequence, TextIO

from graphify_brain import label as labelling
from graphify_brain.label import SQL_VARS, _chunks, _marks
from graphify_brain.plan import envelope

#: The modes with a model in the loop. `free` is decided by its rule alone and is never
#: read here — that is the whole point of it.
MODES = ("hybrid", "full")

#: How many calls one pattern may be handed in one run. Not a money bound — the caps are
#: that, and they are checked batch by batch — but a memory one: every candidate's
#: transcript is read before the first batch is priced, and a full-mode pattern over a
#: fortnight of a busy org would otherwise load tens of thousands of them to label three.
#: Twenty-five batches is more than a $1 cap can pay for at any priced model.
CANDIDATE_LIMIT = 500


@dataclass(frozen=True)
class Pattern:
    """A `patterns` row with everything a run needs, and nothing missing.

    Built only for rows that have all of it: `_patterns` skips the rest and says so, so
    every field here is a value rather than a maybe.
    """

    id: int
    org_id: int
    name: str
    criterion: str
    plan: Any
    model: str
    mode: str
    cap_usd: float
    assistant_ids: list[str]


def run(stdin: TextIO, stdout: TextIO, stderr: TextIO, conn: Any) -> None:
    """The whole command: read the budget, read the patterns, spend up to it."""
    payload = _request(stdin)
    envelope(payload, "daily", required={"org", "max_usd"}, optional=set())
    org = _org(payload["org"])
    budget = _budget(payload["max_usd"])

    patterns = _patterns(conn, org, stderr)
    done: list[dict[str, Any]] = []
    spent = 0.0
    stopped: str | None = None

    for i, pattern in enumerate(patterns):
        left = budget - spent
        if left <= 0:
            # The same words `label` uses when it stops, so one search of a job's log
            # finds either kind of stop.
            stopped = "cap"
            print(
                f"cap reached: ${spent:.4f} of ${budget:.4f} spent, "
                f"{len(patterns) - i} patterns not read",
                file=stderr,
                flush=True,
            )
            break
        report = _one(conn, pattern, min(pattern.cap_usd, left), stdout, stderr)
        spent += report["usd"]
        done.append(report)
        print(f"PROGRESS {i + 1}/{len(patterns)}", file=stderr, flush=True)

    print(json.dumps({"usd": round(spent, 6), "patterns": done, "stopped": stopped}), file=stdout)


def _one(conn: Any, pattern: Pattern, budget: float, stdout: TextIO, stderr: TextIO) -> dict[str, Any]:
    """One pattern's reading, inside one budget.

    Every failure is caught and reported rather than raised. A provider that falls over on
    the third of five patterns must not throw away the record of what the first two cost —
    the engine reads that off the last line, and there is no last line after a traceback.
    """
    calls = _candidates(conn, pattern)
    if not calls:
        print(f"pattern {pattern.id} {pattern.name}: nothing new to read", file=stderr, flush=True)
        return _report(pattern, [], 0.0, None, None)

    request = {
        "criterion": pattern.criterion,
        "plan": pattern.plan,
        "call_ids": calls,
        "model": pattern.model,
        "max_usd": budget,
        "pattern_id": pattern.id,
    }
    try:
        got = labelling.label_calls(request, conn, stdout, stderr)
    except Exception as e:  # noqa: BLE001 — reported, for the reason in the docstring
        traceback.print_exc(file=stderr)
        return _report(pattern, [], 0.0, None, f"{type(e).__name__}: {e}")

    _store(conn, pattern.id, got["labels"])
    return _report(pattern, got["labels"], got["usd"], got["stopped"], None)


def _report(
    pattern: Pattern,
    labels: Sequence[dict[str, Any]],
    usd: float,
    stopped: str | None,
    error: str | None,
) -> dict[str, Any]:
    return {
        "pattern": pattern.id,
        "mode": pattern.mode,
        "read": len(labels),
        "matched": sum(1 for x in labels if x["match"]),
        "usd": usd,
        "stopped": stopped,
        "error": error,
    }


def _store(conn: Any, pattern_id: int, labels: Sequence[dict[str, Any]]) -> None:
    """Turn a wave of verdicts into matches.

    A confirmed call gets a `source='llm'` row of its own. In hybrid it will now have two
    rows — the rule's and the model's — and that is one call, which is why the count in
    `engine/src/queries.rs` is over `DISTINCT call_id`.

    A rejected call loses the rule's row. This is what makes the confirmation worth paying
    for: without it the count would be identical before and after, and hybrid mode would be
    a bill for a number that did not move. `engine/src/rules.rs` will not put the row back
    the next time the rule is run, for the same reason.
    """
    matched = [x["call_id"] for x in labels if x["match"]]
    missed = [x["call_id"] for x in labels if not x["match"]]
    if matched:
        conn.executemany(
            "INSERT INTO pattern_matches (pattern_id, call_id, source) VALUES (?, ?, 'llm')",
            [(pattern_id, i) for i in matched],
        )
    for chunk in _chunks(missed, SQL_VARS):
        conn.execute(
            "DELETE FROM pattern_matches WHERE pattern_id = ? AND source = 'rule' "
            f"AND call_id IN ({_marks(chunk)})",
            [pattern_id, *chunk],
        )
    conn.commit()


def _candidates(conn: Any, pattern: Pattern) -> list[str]:
    """The calls this pattern would read, newest first.

    Four conditions, and each one is money. The org and the pattern's assistants are what
    it is about; a call with nothing said on it is a call the model would be paid to say it
    could not read; a call already in `pattern_labels` has been read and paid for once, and
    a model does not get asked the same question twice. In hybrid there is a fifth: the
    rule's own matches, which is what makes it a prefilter rather than a suggestion.
    """
    where = ["c.org_id = ?", "c.transcript IS NOT NULL", "trim(c.transcript) <> ''"]
    args: list[Any] = [pattern.org_id]
    if pattern.assistant_ids:
        where.append(f"c.assistant_id IN ({_marks(pattern.assistant_ids)})")
        args += pattern.assistant_ids
    where.append("c.id NOT IN (SELECT call_id FROM pattern_labels WHERE pattern_id = ?)")
    args.append(pattern.id)
    if pattern.mode == "hybrid":
        where.append(
            "c.id IN (SELECT call_id FROM pattern_matches WHERE pattern_id = ? AND source = 'rule')"
        )
        args.append(pattern.id)
    args.append(CANDIDATE_LIMIT)
    rows = conn.execute(
        f"SELECT c.id FROM calls c WHERE {' AND '.join(where)} "
        "ORDER BY c.created_at DESC LIMIT ?",
        args,
    )
    return [row["id"] for row in rows]


def _patterns(conn: Any, org: int, stderr: TextIO) -> list[Pattern]:
    """The org's model-backed patterns, in id order, skipping the ones that cannot run.

    Skipped and said out loud, not refused. A row half-way through being made is not a
    reason to leave every other pattern in the org unread for the day, and a missing cap in
    particular has to be a skip rather than a default — a cap that can be left out is a cap
    that gets left out.
    """
    marks = ",".join("?" * len(MODES))
    rows = conn.execute(
        "SELECT id, org_id, name, criterion, plan, rule, model, mode, daily_cap_usd, assistant_ids "
        f"FROM patterns WHERE org_id = ? AND mode IN ({marks}) ORDER BY id",
        [org, *MODES],
    ).fetchall()

    ready = []
    for row in rows:
        if (why := _not_ready(row)) is not None:
            print(f"pattern {row['id']} {row['name'] or ''}: {why}, skipped", file=stderr, flush=True)
            continue
        ready.append(
            Pattern(
                id=row["id"],
                org_id=row["org_id"],
                name=row["name"] or f"#{row['id']}",
                criterion=row["criterion"],
                plan=json.loads(row["plan"]),
                model=row["model"],
                mode=row["mode"],
                cap_usd=float(row["daily_cap_usd"]),
                assistant_ids=_assistant_ids(row["assistant_ids"]),
            )
        )
    return ready


def _not_ready(row: Any) -> str | None:
    """Why this row cannot be read against, or `None` if it can."""
    if not (row["criterion"] or "").strip():
        return "no criterion"
    if not row["plan"]:
        return "no plan"
    if not (row["model"] or "").strip():
        return "no model"
    if row["daily_cap_usd"] is None or float(row["daily_cap_usd"]) <= 0:
        return "no daily cap"
    if row["mode"] == "hybrid" and not row["rule"]:
        # In hybrid the rule chooses the calls. Without one there is nothing to prefilter,
        # and reading the whole org would be `full` mode bought by accident.
        return "hybrid with no rule"
    return None


def _assistant_ids(value: Any) -> list[str]:
    """The pattern's assistants, or an empty list for one that is about the whole org.

    A column that will not parse is treated as "not scoped" rather than as an error: the
    scope narrows what is read, so the worst a bad value can do here is widen it to the org
    the pattern already belongs to.
    """
    if not value:
        return []
    try:
        ids = json.loads(value)
    except (TypeError, ValueError):
        return []
    return [i for i in ids if isinstance(i, str) and i.strip()] if isinstance(ids, list) else []


# --- checking the request -------------------------------------------------------------


def _org(value: Any) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"daily: org must be a whole number, not {value!r}")
    return value


def _budget(value: Any) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or value <= 0:
        # No default, for the reason `label._max_usd` gives. The engine works this out
        # from the day's spend and the global cap; a run that arrived without one would be
        # a daily run with no ceiling, which is the one thing D-8 forbids.
        raise ValueError(f"daily: max_usd must be a positive number, not {value!r}")
    return float(value)


def _request(stdin: TextIO) -> Any:
    try:
        return json.loads(stdin.readline())
    except json.JSONDecodeError as e:
        raise ValueError(f"stdin is not JSON: {e}") from e
