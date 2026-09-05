"""The two steps that happen before any money is spent.

`plan` reads a criterion and says back what it understood. `clarify` reads that plan plus
the analyst's answers and says it back again, better. Neither reads a transcript, so
neither can be wrong about a call — the only thing they can be wrong about is what the
analyst meant, which is exactly what the plan table is for showing them.

Neither judges its own answer. A plan that comes back at 0.4 confidence with three
questions is a fine plan and is printed as it stands; the gate that will not spend money
below 0.95 lives in the wizard, where the person who would be spending it can see it.

Both cost money, and both say so. Neither parks on a go — the Send button is the go, and
a second click to approve four tenths of a cent would teach an analyst to click through
prices. What each one does instead is the other half of the rule: the ceiling goes out as
`ESTIMATE` before the model is touched and is refused outright when it is over the
caller's cap, and what the provider actually charged comes back as `usd` beside the plan.
The ceiling is what the cap is checked against; the collector's number is what is booked.
Both are priced at the model the request names, which is also the model the call runs on:
`plan.baml` declares a client the way `label.baml` and `ask.baml` do, and every one of
them is overridden per call. A price quoted for a model that is not the one being called
is worse than no price at all.

Every model call in this module goes through `client()`, and that is the only way in. A
test replaces that one function and has replaced every call. `charged()` is the second
seam and exists for the other half of the same problem: a fake client returns a canned
answer, and no fake can know what a call it never made was billed for.
"""

from __future__ import annotations

import json
import sys
from typing import Any, Callable

from graphify_brain import cost

#: The rule DSL as the model is told about it.
#:
#: Not an input from the caller. There is one rule DSL, `engine/src/rules.rs` is what
#: decides what it means, and a plan built against a DSL description somebody typed into
#: a request would promise conditions the engine cannot check. It is passed as a function
#: argument rather than written into the prompt so that one test can assert the model was
#: shown it, and so `SynthesizeRule` can be shown the same words in S-24.
#:
#: The last paragraph is the load-bearing one: `expressible` means nothing unless the
#: model has been told where the edge is.
DSL = """\
A rule is one JSON object. Every key is optional, and a key that is absent asks nothing.

  any_phrases      list of strings    a line contains any one of these
  regex            list of strings    a line matches any one of these
  speaker          user | bot | any   whose lines the phrases and regexes are read on
  ended_reasons    list of strings    the call ended for any one of these raw reasons
  ended_groups     list of strings    the call ended in any one of these groups
  tool_called      list of strings    the assistant called any one of these tools
  tool_not_called  list of strings    the assistant called none of these tools
  tool_failed      true | false       some tool call failed / none did
  transferred      true | false       the call was handed to a person / was not
  min_duration_s   number             the call lasted at least this long
  max_duration_s   number             the call lasted at most this long

The ended groups are: customer, assistant, llm-error, tts-error, stt-error,
transfer-error, transport, timeout, start-error, other, unknown.

A call matches when (any phrase OR any regex hits a line spoken by `speaker` — both lists
empty counts as a hit) AND every other condition that is set holds. A value nobody
recorded is unknown and satisfies nothing: a call with no transcript matches no rule
about words, and a call whose transfer nobody recorded matches neither transferred true
nor transferred false.

What the DSL cannot do: count turns, measure a silence, read a tone, judge whether an
answer was correct, compare one call against another, or look at anything beyond the
transcript, the ended reason, the tool calls, the transfer and the duration. A plan that
needs any of that is not expressible.
"""

#: Characters per token, for the ceiling. `label.py` carries the same figure for the same
#: reason — English prose runs nearer four, so three over-counts, which is the direction an
#: estimate guarding a cap has to err. Deliberately its own constant and not a shared one:
#: these are two independent ceilings over two different prompts, and one being re-measured
#: is no reason for the other to move.
CHARS_PER_TOKEN = 3

#: The instructions and the output schema around the arguments, measured from the rendered
#: request and rounded up. `test_plan.py` renders both prompts and fails if either has
#: grown past this, so the ceiling cannot quietly stop being one when a prompt is edited.
FIXED_PROMPT_CHARS = 2_400

#: The `max_tokens` BAML sends with every call. This is what makes the output half of the
#: ceiling a bound rather than a guess: no answer can be longer than this, so no answer can
#: cost more output than the cap check allowed for.
MAX_OUTPUT_TOKENS = 4_096


def client() -> Any:
    """The generated BAML client.

    Imported here rather than at the top of the file for two reasons. `baml_client/` is
    generated and never committed, so importing it at module scope would make
    `graphify-brain version` depend on somebody having run `baml-cli generate`. And this
    function is the seam the tests replace: swap it and every model call in this module
    is swapped, with no second way through.
    """
    from baml_client.sync_client import b

    return b


def plan(payload: dict[str, Any]) -> dict[str, Any]:
    """`{criterion, model, max_usd, system_prompt?}` in, a plan and what it cost out."""
    envelope(
        payload,
        "plan",
        required={"criterion", "model", "max_usd"},
        optional={"system_prompt"},
    )
    criterion = required_text(payload, "criterion")
    model = cost.model_name(payload["model"], "plan")
    cap = max_usd(payload["max_usd"], "plan")
    prompt = payload.get("system_prompt")
    # An assistant with an empty prompt and an assistant whose prompt nobody read are the
    # same absence to the model, and `None` is what skips the prompt block entirely.
    system_prompt = prompt.strip() if isinstance(prompt, str) and prompt.strip() else None

    afford(plan_usd(criterion, system_prompt, model), cap, "plan")

    # Every argument is built before the client is reached for, so a bad input is refused
    # with the model still untouched. Not a style preference: `client().PlanPattern(...)`
    # would resolve the function first and then evaluate the arguments, which reads like
    # a call that has already begun.
    from baml_py import Collector

    collector = Collector()
    result = client().with_options(
        client=cost.CLIENTS[model], collector=collector
    ).PlanPattern(criterion=criterion, system_prompt=system_prompt, dsl=DSL)
    return {**result.model_dump(), "usd": round(charged(collector, model), 6)}


def clarify(payload: dict[str, Any]) -> dict[str, Any]:
    """`{criterion, plan, answers, model, max_usd}` in, the whole plan back out.

    The register's table says the inputs are "plan + user answers". Two more go in, both
    for the same reason: without them the model is grading an answer it cannot check.

    The **criterion** — a `Plan` holds rows, questions and a reason, and none of those is
    the sentence the analyst wrote, which is the thing an answer has to be judged against.

    The **DSL** — an answer can add a row, and a new row can be one the DSL cannot check.
    A `clarify` that could not see the DSL could only carry the old `expressible` forward,
    and the wizard's gate opens on that flag. The money that gate is protecting is spent
    in the next step.
    """
    envelope(
        payload,
        "clarify",
        required={"criterion", "plan", "answers", "model", "max_usd"},
        optional=set(),
    )
    from baml_client import types

    criterion = required_text(payload, "criterion")
    model = cost.model_name(payload["model"], "clarify")
    cap = max_usd(payload["max_usd"], "clarify")
    answers = payload["answers"]
    if not isinstance(answers, list) or not answers:
        raise ValueError("clarify: answers is empty, so there is nothing to revise")
    # `model_validate` rather than a hand-rolled check: these two classes are generated
    # from `plan.baml`, so a plan that does not fit is a plan this brain did not write,
    # and pydantic's error already names the field that is wrong. It raises a
    # `ValidationError`, which is a `ValueError`, so the CLI reports it like any other
    # bad input.
    prior = types.Plan.model_validate(payload["plan"])
    given = [types.Answer.model_validate(a) for a in answers]

    afford(clarify_usd(criterion, prior, given, model), cap, "clarify")

    from baml_py import Collector

    collector = Collector()
    result = client().with_options(
        client=cost.CLIENTS[model], collector=collector
    ).ClarifyPattern(criterion=criterion, plan=prior, answers=given, dsl=DSL)
    return {**result.model_dump(), "usd": round(charged(collector, model), 6)}


def plan_usd(criterion: str, system_prompt: str | None, model: str) -> float:
    """USD for one `plan`, at the ceiling. What the cap is checked against."""
    chars = FIXED_PROMPT_CHARS + len(DSL) + len(criterion) + len(system_prompt or "")
    return cost.estimate(chars // CHARS_PER_TOKEN, MAX_OUTPUT_TOKENS, model)


def clarify_usd(criterion: str, prior: Any, given: list[Any], model: str) -> float:
    """USD for one `clarify`, at the ceiling.

    The plan is counted as the JSON it arrived as rather than as BAML renders it into the
    prompt. The two are within a few dozen characters of each other and `FIXED_PROMPT_CHARS`
    is measured with room over the larger, which is the only way the difference is allowed
    to fall.
    """
    chars = (
        FIXED_PROMPT_CHARS
        + len(DSL)
        + len(criterion)
        + len(prior.model_dump_json())
        + sum(len(a.question) + len(a.answer) for a in given)
    )
    return cost.estimate(chars // CHARS_PER_TOKEN, MAX_OUTPUT_TOKENS, model)


def afford(usd: float, cap: float, name: str) -> None:
    """Say the price, then refuse it if it is over the cap.

    Printed first and whatever happens next: `ESTIMATE` is how the engine learns what a
    job was quoted, and a message refused for being too expensive is exactly the one whose
    price is worth having in the log. Refusing is a `ValueError`, so the CLI reports it the
    way it reports any other bad input — before the model is touched, which is the half of
    the rule a message that never parks has to keep.
    """
    print(f"ESTIMATE {usd:.4f}", file=sys.stdout, flush=True)
    if usd > cap:
        raise ValueError(
            f"{name}: this message could cost up to ${usd:.4f}, over the ${cap:.4f} cap"
        )


def charged(collector: Any, model: str) -> float:
    """What the provider says the call it just made actually cost.

    Its own function because it is the seam a test replaces. A fake client returns a
    canned answer, and no fake can know what a call it never made was billed for — so
    "what did the model say" and "what did it cost" have to be two questions with two
    answers, or every test in `test_plan.py` would be asserting a price it invented.
    """
    usage = collector.last.usage
    return cost.estimate(usage.input_tokens or 0, usage.output_tokens or 0, model)


def max_usd(value: Any, name: str) -> float:
    """The caller's ceiling for this one message.

    Worded exactly as `label._max_usd` and `synth._max_usd` word it, and a fourth copy
    rather than an import because `label` already imports `envelope` from here and the
    other direction would close the circle.
    """
    if isinstance(value, bool) or not isinstance(value, (int, float)) or value <= 0:
        # No default. A cap that a caller can leave out is a cap that gets left out, and
        # "must not exceed max_usd" means nothing when there is no max_usd.
        raise ValueError(f"{name}: max_usd must be a positive number, not {value!r}")
    return float(value)


def run(fn: Callable[[dict[str, Any]], dict[str, Any]], stdin: str) -> str:
    """One line of JSON in, one line of JSON out.

    Raises `ValueError` both for a stdin that is not JSON and for one whose fields are
    not the fields `fn` has.
    """
    try:
        payload = json.loads(stdin)
    except json.JSONDecodeError as e:
        raise ValueError(f"stdin is not JSON: {e}") from e
    return json.dumps(fn(payload))


def envelope(payload: Any, name: str, required: set[str], optional: set[str]) -> None:
    """Refuse an input whose fields are not the fields this function has.

    Public, and named without an underscore, because `label.py` needs the same refusal
    worded the same way. Two copies of it would be two places for the wording to drift.

    An unknown key is refused rather than ignored for the same reason the engine refuses
    one in a rule: a `criteria` where `criterion` was meant would otherwise reach the
    model as no criterion at all, and the model would invent one and sound sure about it.
    """
    if not isinstance(payload, dict):
        raise ValueError(f"{name}: input must be a JSON object")
    keys = set(payload)
    if missing := required - keys:
        raise ValueError(f"{name}: input is missing {', '.join(sorted(missing))}")
    if unknown := keys - required - optional:
        raise ValueError(f"{name}: input has no field {', '.join(sorted(unknown))}")


def required_text(payload: dict[str, Any], key: str) -> str:
    value = payload.get(key)
    if not isinstance(value, str) or not value.strip():
        # An empty criterion is not a criterion the model should guess at. It would
        # answer with a plan for some plausible question and a confidence about it.
        raise ValueError(f"{key} must be a non-empty string")
    return value.strip()
