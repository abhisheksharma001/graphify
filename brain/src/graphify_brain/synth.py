"""Writing the rule that replaces the model, and finding out how good it is.

This is the step the whole wizard exists for. A model has read the calls once and said
which ones matched; what comes out of here is a rule in the DSL that reaches the same
verdicts on its own. From then on every call is classified by `engine/src/rules.rs` for
nothing, which is what a `free`-mode pattern is: one whose model has already been paid for
and dismissed.

**Nothing the model returns is executed.** The rule is data. It reaches the engine as a
file passed to `graphify rule-check` by path, in an argument list with no shell anywhere
near it, and `engine/src/rules.rs` is the only thing that ever reads one. Its regexes are
compiled by the `regex` crate, which has no backtracking, and never by Python. A rule that
arrived full of shell metacharacters would be a rule full of shell metacharacters that
matched nothing.

**The engine is the only thing that says what a rule means.** `agreement` is not computed
from a Python reimplementation of the DSL — it is computed from the ids `graphify
rule-check` printed. A second implementation that drifted would report a number about a
rule nobody runs.

**The refinement has to earn its place.** When agreement comes in under `MIN_AGREEMENT`,
one `RefineRule` call gets the disagreements, and the rule it returns is checked the same
way. It is kept only if it agrees on more calls than the one it replaced. The model is not
trusted to have improved anything; there is a number that says whether it did.
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Sequence, TextIO

from graphify_brain import cost
from graphify_brain.label import CHARS_PER_TOKEN, MAX_OUTPUT_TOKENS
from graphify_brain.plan import DSL, envelope, required_text

#: Below this, one `RefineRule` call is made. The register's number. It is a fraction of
#: calls agreed on, not a fraction of matches found: a rule can score 0.9 by matching
#: nothing at all when only a tenth of the sample matched, which is why `agreement` is
#: reported next to the counts that produced it and not on its own.
MIN_AGREEMENT = 0.85

#: How many disagreements `RefineRule` is shown. All of them would make the second call
#: bigger than the first for a rule that is badly wrong, which is exactly the case where
#: the extra tokens buy the least — a rule disagreeing on eighty calls needs rewriting,
#: not annotating.
MAX_DISAGREEMENTS = 30

#: The prompts around the criterion, the plan and the labels, measured from a rendered
#: request and rounded up. `test_synth.py` re-measures both and fails if either has grown
#: past its number, so an edit to a prompt cannot quietly make every estimate too small.
SYNTHESIZE_PROMPT_CHARS = 3_600
REFINE_PROMPT_CHARS = 3_200

#: The engine binary. `graphify` on PATH unless told otherwise — the same shape S-25 uses
#: to find the brain from the engine, pointing the other way.
ENGINE_ENV = "GRAPHIFY_BIN"
ENGINE_DEFAULT = "graphify"


@dataclass(frozen=True)
class Job:
    """A checked request. Nothing here is still in doubt, and no money has been spent."""

    criterion: str
    plan: Any
    plan_text: str
    labels: list[dict[str, Any]]
    """`{call_id, match, evidence}`, exactly as `label` returned them."""
    model: str
    max_usd: float
    org_id: int
    name: str
    assistant_ids: list[str]
    subjects: list[dict[str, Any]]
    """The same calls in the shape `rule-check --calls` reads."""


def client() -> Any:
    """The generated BAML client — the same seam as `plan.client` and `label.client`."""
    from baml_client.sync_client import b

    return b


def run(stdin: TextIO, stdout: TextIO, stderr: TextIO, conn: Any) -> None:
    """Read the request, show the price, write the rule, measure it, store it.

    No `GO` handshake, unlike `label`. This is one model call on a few hundred short
    quotes, it follows a labelling the analyst has already paid for in the same click, and
    a second confirmation in the middle of one flow trains people to click through both.
    The price is still shown and the cap is still checked, because a model call with no
    published cost is the thing the spec forbids.
    """
    job = prepare(_request(stdin), conn)
    ceiling = estimate(job)
    print(f"ESTIMATE {ceiling:.4f}", file=stdout, flush=True)
    if ceiling > job.max_usd:
        raise ValueError(
            f"synthesize: this would cost up to ${ceiling:.4f} and the cap is "
            f"${job.max_usd:.4f}; nothing was sent"
        )
    print(json.dumps(_synthesize(job, conn, stderr)), file=stdout)


def prepare(payload: Any, conn: Any) -> Job:
    """Check the request and read the calls it labelled. Raises `ValueError` for anything
    wrong with either, with no model touched."""
    envelope(
        payload,
        "synthesize",
        required={"criterion", "plan", "labels", "model", "max_usd", "org_id", "name"},
        optional={"assistant_ids"},
    )
    from baml_client import types

    criterion = required_text(payload, "criterion")
    plan = types.Plan.model_validate(payload["plan"])
    name = required_text(payload, "name")
    model = cost.model_name(payload["model"], "synthesize")
    max_usd = _max_usd(payload["max_usd"])
    org_id = _whole(payload["org_id"], "org_id")
    assistants = _assistant_ids(payload.get("assistant_ids", []))
    labels = _labels(payload["labels"])

    ids = [x["call_id"] for x in labels]
    subjects = _subjects(conn, ids)
    if missing := [i for i in ids if i not in subjects]:
        raise ValueError(
            f"synthesize: {len(missing)} of these labelled calls are not in the database, "
            f"the first is {missing[0]}"
        )

    return Job(
        criterion,
        plan,
        plan.model_dump_json(),
        labels,
        model,
        max_usd,
        org_id,
        name,
        assistants,
        [subjects[i] for i in ids],
    )


def estimate(job: Job) -> float:
    """USD for the whole job at the ceiling: both calls, because the refinement is the
    worst case and a cap is only a cap against the worst case."""
    return _synthesize_usd(job) + _refine_usd(job)


def _synthesize_usd(job: Job) -> float:
    evidence = sum(len(x["evidence"]) + 12 for x in job.labels)
    chars = SYNTHESIZE_PROMPT_CHARS + len(job.criterion) + len(job.plan_text) + len(DSL) + evidence
    return cost.estimate(chars // CHARS_PER_TOKEN, MAX_OUTPUT_TOKENS, job.model)


def _refine_usd(job: Job) -> float:
    # The refinement carries at most `MAX_DISAGREEMENTS` quotes and the rule the first call
    # returned. The rule is priced at its own output ceiling, since that is the most it can
    # have been.
    worst = sorted((len(x["evidence"]) + 60 for x in job.labels), reverse=True)[:MAX_DISAGREEMENTS]
    chars = REFINE_PROMPT_CHARS + len(job.criterion) + len(job.plan_text) + len(DSL) + sum(worst)
    return cost.estimate(chars // CHARS_PER_TOKEN + MAX_OUTPUT_TOKENS, MAX_OUTPUT_TOKENS, job.model)


def synthesize_rule(job: Job) -> tuple[Any, float]:
    """The first model call: labels and evidence in, a rule and a chart out."""
    from baml_py import Collector

    from baml_client import types

    seen = [types.LabelForRule(match=x["match"], evidence=x["evidence"]) for x in job.labels]
    collector = Collector()
    got = client().with_options(client=cost.CLIENTS[job.model], collector=collector).SynthesizeRule(
        criterion=job.criterion, plan=job.plan, labels=seen, dsl=DSL
    )
    return got, _spent(collector, job.model)


def refine_rule(job: Job, rule: Any, disagreements: Sequence[dict[str, Any]]) -> tuple[Any, float]:
    """The second model call, made only when the first rule agreed on too few calls.

    Returns a `Refinement` — the rule and a reason of its own. The synthesis reason
    describes the rule this one replaces, and printing it beside the new rule would be an
    explanation of something nobody is running.
    """
    from baml_py import Collector

    from baml_client import types

    told = [
        types.Disagreement(labelled=d["labelled"], matched=d["matched"], evidence=d["evidence"])
        for d in disagreements[:MAX_DISAGREEMENTS]
    ]
    collector = Collector()
    got = client().with_options(client=cost.CLIENTS[job.model], collector=collector).RefineRule(
        criterion=job.criterion, plan=job.plan, rule=rule, disagreements=told, dsl=DSL
    )
    return got, _spent(collector, job.model)


def _spent(collector: Any, model: str) -> float:
    usage = collector.last.usage
    return cost.estimate(usage.input_tokens or 0, usage.output_tokens or 0, model)


def _synthesize(job: Job, conn: Any, stderr: TextIO) -> dict[str, Any]:
    """Write the rule, measure it, refine it if it needs it, store what wins."""
    print("PROGRESS 1/3", file=stderr, flush=True)
    first, spent = synthesize_rule(job)
    rule, reason = first.rule, first.reason

    print("PROGRESS 2/3", file=stderr, flush=True)
    scored = _score(job, rule)
    refined = False
    if scored.agreement < MIN_AGREEMENT:
        second, usd = refine_rule(job, rule, scored.disagreements)
        spent += usd
        again = _score(job, second.rule)
        # Kept only if it is actually better. A second opinion that agreed on fewer calls
        # is a second opinion, not an improvement, and the first rule was already paid for.
        if again.agreement > scored.agreement:
            rule, reason, scored, refined = second.rule, second.reason, again, True
        else:
            print(
                f"refinement kept the original: {again.agreement:.3f} against "
                f"{scored.agreement:.3f}",
                file=stderr,
                flush=True,
            )

    print("PROGRESS 3/3", file=stderr, flush=True)
    pattern_id = _store(conn, job, rule, first.chart, scored)
    return {
        "pattern_id": pattern_id,
        "rule": _rule_json(rule),
        "chart": {"kind": first.chart.kind.value, "title": first.chart.title},
        "agreement": round(scored.agreement, 4),
        "agreed": scored.agreed,
        "of": len(job.labels),
        "matched_by_rule": len(scored.matched),
        "matched_by_model": sum(1 for x in job.labels if x["match"]),
        "refined": refined,
        "reason": reason,
        "usd": round(spent, 6),
        "model": job.model,
    }


# --- what the engine says the rule means ----------------------------------------------


@dataclass(frozen=True)
class Scored:
    agreement: float
    agreed: int
    matched: set[str]
    disagreements: list[dict[str, Any]]


def _score(job: Job, rule: Any) -> Scored:
    """Run the rule over the labelled calls and count where it agrees.

    `agreement` is over every call in the sample, not over the matches: a call both sides
    said no to is a call they agree about. The register's worked example — forty labelled
    matches, thirty-eight of them found plus two others out of two hundred and fifty — is
    246/250, and it is 246 because the two hundred and eight nobody matched count.
    """
    matched = rule_check(_rule_json(rule), job.subjects)
    agreed = 0
    disagreements = []
    for label in job.labels:
        by_rule = label["call_id"] in matched
        if by_rule == label["match"]:
            agreed += 1
        else:
            disagreements.append(
                {"labelled": label["match"], "matched": by_rule, "evidence": label["evidence"]}
            )
    return Scored(agreed / len(job.labels), agreed, matched, disagreements)


def rule_check(rule: dict[str, Any], subjects: Sequence[dict[str, Any]]) -> set[str]:
    """The ids `graphify rule-check` printed for this rule over these calls.

    A subprocess with an argument list and no shell. The rule reaches the engine as a file
    it opens by path, so there is nothing in it — a quote, a semicolon, a backtick — that
    can be anything but bytes in a JSON document. This function is the whole of "must not
    execute anything returned by the model", and it is the only place a rule is run.
    """
    binary = os.environ.get(ENGINE_ENV) or ENGINE_DEFAULT
    with tempfile.TemporaryDirectory(prefix="graphify-rule-") as tmp:
        rule_file = Path(tmp) / "rule.json"
        calls_file = Path(tmp) / "calls.json"
        rule_file.write_text(json.dumps(rule))
        calls_file.write_text(json.dumps(list(subjects)))
        # A list, never a string, and no shell: the rule is a path the engine opens, so
        # nothing in it — a quote, a semicolon, a backtick — is ever anything but bytes.
        done = subprocess.run(
            [binary, "rule-check", "--rule", str(rule_file), "--calls", str(calls_file)],
            capture_output=True,
            text=True,
            check=False,
        )
        if done.returncode != 0:
            # The engine refused it — an unknown key, a regex it would not compile, a
            # speaker that is not a speaker. Its own words say which, and they are the
            # only opinion on the subject that counts. The temp path it names is scrubbed
            # first: the file is gone by the time anybody reads this, and a path that
            # cannot be opened in an error message is a minute of somebody's life.
            said = done.stderr.strip().removeprefix("Error: ").replace(str(rule_file), "the rule")
            raise ValueError(f"synthesize: the engine refused the rule — {said}")
    return {line.strip() for line in done.stdout.splitlines() if line.strip()}


def _rule_json(rule: Any) -> dict[str, Any]:
    """The rule as `engine/src/rules.rs` reads it.

    `exclude_none`, because the Rust `Rule` takes a missing key as "do not ask" but cannot
    read a `null` into a `Vec`. The lists are never None — the BAML class requires them —
    so what this drops is only the scalars nobody set.
    """
    return rule.model_dump(exclude_none=True)


# --- storing ---------------------------------------------------------------------------


def _store(conn: Any, job: Job, rule: Any, chart: Any, scored: Scored) -> int:
    """Write the `patterns` row, then attach every label to it.

    The pattern is stored in `free` mode with whatever `daily_cap_usd` the schema defaults
    to, which is the whole point of having got here: a free pattern is decided by its rule
    alone and costs nothing to run again. S-27's mode select is what changes that, in front
    of a person who can see the cap they are turning on.

    The labels land here rather than in S-23 because this is the first moment there is a
    pattern for them to belong to, and `rule_match` is filled in from the same `rule-check`
    that produced the agreement figure — so the two can never come to disagree.
    """
    cursor = conn.execute(
        "INSERT INTO patterns (org_id, name, criterion, assistant_ids, plan, rule, chart, "
        "model, mode, daily_cap_usd, sample_size, agreement, created_at) "
        "VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'free', 1.0, ?, ?, ?)",
        (
            job.org_id,
            job.name,
            job.criterion,
            json.dumps(job.assistant_ids),
            job.plan_text,
            json.dumps(_rule_json(rule)),
            json.dumps({"kind": chart.kind.value, "title": chart.title}),
            job.model,
            len(job.labels),
            scored.agreement,
            _now(),
        ),
    )
    pattern_id = cursor.lastrowid
    conn.executemany(
        "INSERT INTO pattern_labels (pattern_id, call_id, llm_match, rule_match, evidence) "
        "VALUES (?, ?, ?, ?, ?)",
        [
            (pattern_id, x["call_id"], int(x["match"]), int(x["call_id"] in scored.matched), x["evidence"])
            for x in job.labels
        ],
    )
    conn.commit()
    return pattern_id


def _now() -> str:
    """`engine/src/lib.rs`'s `now()`, spelled the same: RFC 3339, milliseconds, `Z`."""
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


# --- reading the calls ------------------------------------------------------------------


def _subjects(conn: Any, ids: Sequence[str]) -> dict[str, dict[str, Any]]:
    """The labelled calls in the shape `rule-check --calls` reads.

    Its own query rather than `label`'s, because it is its own projection: the labeller
    needed a sentence a model could read and this needs the fields a rule can ask about,
    with `tool_calls` as the pairs `engine/src/rules.rs` expects rather than two lists.
    """
    from graphify_brain.label import SQL_VARS, _chunks, _marks

    tools: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for chunk in _chunks(ids, SQL_VARS):
        for row in conn.execute(
            f"SELECT call_id, name, failed FROM tool_calls WHERE call_id IN ({_marks(chunk)})",
            list(chunk),
        ):
            tools[row["call_id"]].append(
                {"name": row["name"], "failed": None if row["failed"] is None else bool(row["failed"])}
            )

    subjects = {}
    for chunk in _chunks(ids, SQL_VARS):
        for row in conn.execute(
            "SELECT id, transcript, ended_reason, ended_group, transferred, duration_s "
            f"FROM calls WHERE id IN ({_marks(chunk)})",
            list(chunk),
        ):
            subjects[row["id"]] = {
                "id": row["id"],
                "transcript": row["transcript"],
                "ended_reason": row["ended_reason"],
                "ended_group": row["ended_group"],
                # A NULL stays a NULL all the way to the engine, which treats unknown as
                # satisfying nothing. Coercing it to false here would make every call
                # nobody recorded a transfer for match `transferred: false`.
                "transferred": None if row["transferred"] is None else bool(row["transferred"]),
                "duration_s": row["duration_s"],
                "tool_calls": tools[row["id"]],
            }
    return subjects


# --- checking the request ---------------------------------------------------------------


def _max_usd(value: Any) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or value <= 0:
        raise ValueError(f"synthesize: max_usd must be a positive number, not {value!r}")
    return float(value)


def _whole(value: Any, key: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"synthesize: {key} must be a whole number, not {value!r}")
    return value


def _assistant_ids(value: Any) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(i, str) or not i.strip() for i in value):
        raise ValueError("synthesize: assistant_ids must be a list of non-empty strings")
    return [i.strip() for i in value]


def _labels(value: Any) -> list[dict[str, Any]]:
    """`label`'s output, handed straight back. Refused rather than repaired: these come
    from a job this brain ran, so one that no longer fits is one it did not write."""
    if not isinstance(value, list) or not value:
        raise ValueError("synthesize: labels is empty, so there is nothing to generalise from")
    out = []
    for label in value:
        if not isinstance(label, dict) or set(label) != {"call_id", "match", "evidence"}:
            raise ValueError(f"synthesize: every label must be {{call_id, match, evidence}}, not {label!r}")
        if not isinstance(label["call_id"], str) or not label["call_id"].strip():
            raise ValueError("synthesize: every label needs a call id")
        if not isinstance(label["match"], bool):
            raise ValueError(f"synthesize: match must be true or false, not {label['match']!r}")
        if not isinstance(label["evidence"], str):
            raise ValueError("synthesize: every label needs its evidence")
        out.append({"call_id": label["call_id"].strip(), "match": label["match"], "evidence": label["evidence"]})
    ids = [x["call_id"] for x in out]
    if len(set(ids)) != len(ids):
        raise ValueError("synthesize: labels holds the same call twice")
    if not any(x["match"] for x in out):
        # A rule written from nothing but non-matches is a rule with nothing to key on, and
        # the model will invent something that scores 1.0 by matching no call ever.
        raise ValueError("synthesize: no call was labelled a match, so there is no pattern to write")
    return out


def _request(stdin: TextIO) -> Any:
    try:
        return json.loads(stdin.read())
    except json.JSONDecodeError as e:
        raise ValueError(f"stdin is not JSON: {e}") from e
