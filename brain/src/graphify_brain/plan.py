"""The two steps that happen before any money is spent.

`plan` reads a criterion and says back what it understood. `clarify` reads that plan plus
the analyst's answers and says it back again, better. Neither reads a transcript, so
neither can be wrong about a call — the only thing they can be wrong about is what the
analyst meant, which is exactly what the plan table is for showing them.

Neither judges its own answer. A plan that comes back at 0.4 confidence with three
questions is a fine plan and is printed as it stands; the gate that will not spend money
below 0.95 lives in the wizard, where the person who would be spending it can see it.

Every model call in this module goes through `client()`, and that is the only way in. A
test replaces that one function and has replaced every call.
"""

from __future__ import annotations

import json
from typing import Any, Callable

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
    """`{criterion, system_prompt?}` in, a plan out."""
    envelope(payload, "plan", required={"criterion"}, optional={"system_prompt"})
    criterion = required_text(payload, "criterion")
    prompt = payload.get("system_prompt")
    # An assistant with an empty prompt and an assistant whose prompt nobody read are the
    # same absence to the model, and `None` is what skips the prompt block entirely.
    system_prompt = prompt.strip() if isinstance(prompt, str) and prompt.strip() else None

    # Every argument is built before the client is reached for, so a bad input is refused
    # with the model still untouched. Not a style preference: `client().PlanPattern(...)`
    # would resolve the function first and then evaluate the arguments, which reads like
    # a call that has already begun.
    result = client().PlanPattern(criterion=criterion, system_prompt=system_prompt, dsl=DSL)
    return result.model_dump()


def clarify(payload: dict[str, Any]) -> dict[str, Any]:
    """`{criterion, plan, answers}` in, the whole plan back out.

    The register's table says the inputs are "plan + user answers". Two more go in, both
    for the same reason: without them the model is grading an answer it cannot check.

    The **criterion** — a `Plan` holds rows, questions and a reason, and none of those is
    the sentence the analyst wrote, which is the thing an answer has to be judged against.

    The **DSL** — an answer can add a row, and a new row can be one the DSL cannot check.
    A `clarify` that could not see the DSL could only carry the old `expressible` forward,
    and the wizard's gate opens on that flag. The money that gate is protecting is spent
    in the next step.
    """
    envelope(payload, "clarify", required={"criterion", "plan", "answers"}, optional=set())
    from baml_client import types

    criterion = required_text(payload, "criterion")
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

    result = client().ClarifyPattern(criterion=criterion, plan=prior, answers=given, dsl=DSL)
    return result.model_dump()


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
