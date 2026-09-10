"""The bound on the wire, for every function on every client.

`test_cost.py` proves that every client in `baml_src/clients.baml` has a *price*. This
file proves that every one of them has a *ceiling*, which is the other half of the same
number: an estimate is tokens times a rate, and the spec forbids calling a model without
showing what it will cost and forbids the daily modes going over a cap. The input half of
that estimate is a count of characters and can only be over-counted. The output half is
the flat `MAX_OUTPUT_TOKENS`, and it is only a ceiling because the request says so.

It was not saying so. No client block named `max_tokens`, and BAML filled a default in
per provider: the anthropic Messages API requires the field so a number went out, and the
openai request left with `messages` and `model` and nothing else — which that API reads
as the model's own limit. The three assertions the suite already had could not see it,
because all three render the *declared* client and every function in `baml_src/` declares
`client Sonnet` while every call site overrides it with the model the analyst picked.

So both lists here are harvested rather than typed. A seventh function or a fourth client
is red until somebody says what bounds its output, which is the only part of this that
outlives the defect.

Nothing here touches the network. `b.request.X(...)` builds the body BAML would send and
does not send it, so there is no key, no spend, and no provider involved.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Any

import pytest

from baml_client import types
from baml_client.sync_client import b
from graphify_brain import ask as asking
from graphify_brain import label as labelling
from graphify_brain import plan as planning
from graphify_brain import synth as synthesis

BAML_SRC = Path(__file__).resolve().parents[1] / "baml_src"


def a_plan() -> Any:
    return types.Plan(rows=[], questions=[], confidence=1.0, expressible=True, reason="")


def a_call() -> Any:
    return types.CallToLabel(n=1, facts="", transcript="")


def a_rule() -> Any:
    return types.Rule(
        any_phrases=[],
        regex=[],
        speaker="user",
        ended_reasons=[],
        ended_groups=[],
        tool_called=[],
        tool_not_called=[],
        tool_failed=None,
        transferred=None,
        min_duration_s=None,
        max_duration_s=None,
    )


#: Every function BAML can be asked for, the module whose estimate prices it, and the
#: smallest arguments that render. The arguments are here because the functions do not
#: share a signature and a harvest cannot invent one; the module is here because the
#: number being asserted is that module's own constant, so two modules drifting apart is
#: red rather than averaged away. Checked against the harvest below in both directions.
RENDERS: dict[str, tuple[Any, dict[str, Any]]] = {
    "PlanPattern": (planning, dict(criterion="x", system_prompt=None, dsl="")),
    "ClarifyPattern": (planning, dict(criterion="x", plan=a_plan(), answers=[], dsl="")),
    "LabelBatch": (labelling, dict(criterion="x", plan=a_plan(), calls=[a_call()])),
    "AskAnalysis": (asking, dict(question="q", stats="{}", calls=[a_call()])),
    "SynthesizeRule": (synthesis, dict(criterion="x", plan=a_plan(), labels=[], dsl="")),
    "RefineRule": (
        synthesis,
        dict(criterion="x", plan=a_plan(), rule=a_rule(), disagreements=[], dsl=""),
    ),
}


def declared_functions() -> set[str]:
    """Every `function` in `baml_src/`, read from the files a person edits.

    Not from `baml_client/`: that directory is generated and never committed, so a harvest
    over it would be a harvest over whatever the last `baml-cli generate` produced rather
    than over what the repository says.
    """
    return {
        name
        for f in BAML_SRC.glob("*.baml")
        for name in re.findall(r"^function\s+(\w+)", f.read_text(), re.MULTILINE)
    }


def declared_clients() -> set[str]:
    """Every `client<llm>` in `clients.baml`, the same way `test_cost.py` reads them."""
    return set(re.findall(r"^client<llm>\s+(\w+)", (BAML_SRC / "clients.baml").read_text(), re.MULTILINE))


def test_every_function_that_can_be_called_is_rendered_here():
    """A function nobody wrote arguments for is a function nobody checked the ceiling of.

    Both directions: an entry left behind by a deleted function is as wrong as a missing
    one, because it is a guard that reads as covering something that is gone.
    """
    assert declared_functions() == set(RENDERS)


@pytest.mark.parametrize("function", sorted(RENDERS))
@pytest.mark.parametrize("client", sorted(declared_clients()))
def test_the_request_carries_the_ceiling_the_estimate_priced(client: str, function: str):
    """The acceptance case, once per function per client.

    The declared client is not the one a run uses: every function in `baml_src/` says
    `client Sonnet` and every call site overrides it with `with_options`. So the client is
    a parameter here, and `GPT` is the case that was wrong.
    """
    module, kwargs = RENDERS[function]
    body = getattr(b.with_options(client=client).request, function)(**kwargs).body.json()

    assert body["max_tokens"] == module.MAX_OUTPUT_TOKENS


def test_every_client_block_writes_its_ceiling_down_rather_than_inheriting_one():
    """The reason the openai request had none, and the only guard that catches the two
    anthropic clients — their default is 4096 today, so dropping the line there changes no
    request and no rendered body. A provider's default is the provider's to move with a
    version bump; the number the quote is built from has to be in the file that is
    committed.

    Whole `client<llm> ... {}` blocks, the way `test_cost.py` reads them, so two ceilings
    in one block cannot stand in for a missing one in the next.
    """
    blocks = re.findall(
        r"client<llm>\s+(\w+)\s*\{(.*?)\n\}", (BAML_SRC / "clients.baml").read_text(), re.DOTALL
    )

    assert {name for name, _ in blocks} == declared_clients()
    assert [name for name, body in blocks if not re.search(r"^\s*max_tokens\s+\d+", body, re.MULTILINE)] == []
