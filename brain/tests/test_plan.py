"""Not one test in this file may call a model.

That is the step's "must not", and it is held two ways. Every test that expects a call
replaces `plan.client` with a `Recorder`, which returns a canned answer and remembers the
arguments. Every test that expects no call replaces it with `Never`, which raises on any
attribute access — so "no model was called" is asserted by the fake rather than by
reading the code and hoping.
"""

import json

import pytest
from typer.testing import CliRunner

from baml_client import types
from graphify_brain import plan as planning
from graphify_brain.cli import app


def a_plan(**over):
    """A believable answer from the model. Override the fields a test is about."""
    fields = {
        "rows": [types.PlanRow(if_="the caller asks for a person", then="counts as a match")],
        "questions": [],
        "confidence": 1.0,
        "expressible": True,
        "reason": "Nothing in the sentence reads two ways.",
    }
    return types.Plan(**(fields | over))


class Recorder:
    """Stands in for the generated BAML client, and keeps what it was asked."""

    def __init__(self, reply):
        self.reply = reply
        self.calls = []

    def PlanPattern(self, **kwargs):  # noqa: N802 — the generated client's name
        self.calls.append(("PlanPattern", kwargs))
        return self.reply

    def ClarifyPattern(self, **kwargs):  # noqa: N802
        self.calls.append(("ClarifyPattern", kwargs))
        return self.reply


class Never:
    """Reaching for any function on this is reaching for a model. That is the failure."""

    def __getattr__(self, name):
        raise AssertionError(f"a test called {name}; no test may call a model")


@pytest.fixture
def recorder(monkeypatch):
    def install(reply):
        fake = Recorder(reply)
        monkeypatch.setattr(planning, "client", lambda: fake)
        return fake

    return install


@pytest.fixture
def never(monkeypatch):
    monkeypatch.setattr(planning, "client", Never)


def run(args, stdin):
    return CliRunner().invoke(app, args, input=stdin)


# --- the plan comes back as the model wrote it ------------------------------------


def test_a_plan_the_model_is_unsure_about_is_printed_unchanged(recorder):
    """The acceptance test. The brain reports; it does not grade. A plan at 0.7 with two
    questions is the wizard's business — it is what puts the questions in the chat — and a
    brain that withheld it would leave the analyst nothing to answer."""
    fake = recorder(
        a_plan(
            confidence=0.7,
            questions=[
                "Does a caller who asks for the manager count?",
                "Does it count when the assistant offers first?",
            ],
        )
    )

    result = run(["plan"], '{"criterion": "calls where the caller asked for a person"}')

    assert result.exit_code == 0
    assert json.loads(result.stdout) == fake.reply.model_dump()
    assert json.loads(result.stdout)["confidence"] == 0.7
    assert len(json.loads(result.stdout)["questions"]) == 2


def test_the_row_condition_is_called_if_underscore_the_whole_way_out(recorder):
    """`if_` in the BAML class, `if_` in the JSON, `if_` in the UI. `if` is a Python
    keyword, so one of those hops would have to rename it, and a field with two names is
    a field somebody reads under the wrong one."""
    recorder(a_plan())

    result = run(["plan"], '{"criterion": "asked for a person"}')

    assert json.loads(result.stdout)["rows"][0]["if_"] == "the caller asks for a person"


# --- what the model is told --------------------------------------------------------


def test_the_model_is_shown_the_dsl_it_has_to_plan_against(recorder):
    fake = recorder(a_plan())

    run(["plan"], '{"criterion": "asked for a person"}')

    name, kwargs = fake.calls[0]
    assert name == "PlanPattern"
    assert kwargs["dsl"] == planning.DSL
    assert "any_phrases" in kwargs["dsl"] and "tool_not_called" in kwargs["dsl"]


def test_the_dsl_says_where_its_edge_is():
    """`expressible` is a question about the boundary, so the boundary has to be in the
    text. Without it the model can only guess, and it will guess yes."""
    assert "cannot do" in planning.DSL
    assert "not expressible" in planning.DSL


def test_an_assistant_prompt_nobody_read_reaches_the_model_as_nothing(recorder):
    """The wizard's "read the agent's prompt" toggle is off, or the assistant has no
    prompt stored. Both are the same absence, and an empty string is not a prompt."""
    fake = recorder(a_plan())

    run(["plan"], '{"criterion": "asked for a person", "system_prompt": "   "}')

    assert fake.calls[0][1]["system_prompt"] is None


def test_an_assistant_prompt_is_handed_over_whole(recorder):
    fake = recorder(a_plan())

    run(["plan"], json.dumps({"criterion": "booked", "system_prompt": "You book trucks.\nUse bookAppointment."}))

    assert fake.calls[0][1]["system_prompt"] == "You book trucks.\nUse bookAppointment."


# --- inputs that are refused, without spending anything ------------------------------


def test_an_empty_criterion_is_refused_before_the_model(never):
    result = run(["plan"], '{"criterion": "   "}')

    assert result.exit_code == 1
    assert "criterion must be a non-empty string" in result.output


def test_a_missing_criterion_is_refused(never):
    result = run(["plan"], "{}")

    assert result.exit_code == 1
    assert "missing criterion" in result.output


def test_a_misspelled_field_is_named_rather_than_dropped(never):
    """`criteria` for `criterion` would otherwise arrive as no criterion at all, and the
    model would answer about some other question with a confidence attached."""
    result = run(["plan"], '{"criterion": "asked for a person", "systemprompt": "x"}')

    assert result.exit_code == 1
    assert "has no field systemprompt" in result.output


def test_stdin_that_is_not_json_exits_one(never):
    result = run(["plan"], "criterion: asked for a person")

    assert result.exit_code == 1
    assert "stdin is not JSON" in result.output


def test_a_db_that_is_not_there_fails_before_the_model(never, tmp_path):
    """`--db` is checked, not read. A path that is wrong is wrong at the first step of
    the wizard rather than three functions later, after the labelling has been paid for."""
    result = run(["plan", "--db", str(tmp_path / "nope.db")], '{"criterion": "asked"}')

    assert result.exit_code == 1
    assert "no graphify database at" in result.output


# --- clarify --------------------------------------------------------------------


def test_clarify_hands_the_model_the_criterion_the_plan_and_the_answers(recorder):
    """The criterion is in there on purpose. A `Plan` holds rows, questions and a reason,
    and none of them is the sentence the analyst wrote — which is the thing an answer has
    to be judged against."""
    fake = recorder(a_plan(confidence=0.96, questions=[]))
    stdin = json.dumps(
        {
            "criterion": "calls where the caller asked for a person",
            "plan": a_plan(confidence=0.7, questions=["Does the manager count?"]).model_dump(),
            "answers": [{"question": "Does the manager count?", "answer": "Yes."}],
        }
    )

    result = run(["clarify"], stdin)

    assert result.exit_code == 0
    name, kwargs = fake.calls[0]
    assert name == "ClarifyPattern"
    assert kwargs["criterion"] == "calls where the caller asked for a person"
    assert kwargs["plan"].questions == ["Does the manager count?"]
    assert kwargs["answers"][0].answer == "Yes."
    assert kwargs["dsl"] == planning.DSL
    assert json.loads(result.stdout)["confidence"] == 0.96


def test_clarify_with_no_answers_is_refused(never):
    stdin = json.dumps({"criterion": "asked for a person", "plan": a_plan().model_dump(), "answers": []})

    result = run(["clarify"], stdin)

    assert result.exit_code == 1
    assert "nothing to revise" in result.output


def test_clarify_refuses_a_plan_that_is_not_one(never):
    """The plan comes back from a UI that got it from here. One that no longer fits the
    class is one this brain did not write, and guessing the missing field is worse."""
    broken = a_plan().model_dump()
    del broken["expressible"]
    stdin = json.dumps({"criterion": "asked", "plan": broken, "answers": [{"question": "q", "answer": "a"}]})

    result = run(["clarify"], stdin)

    assert result.exit_code == 1
    assert "expressible" in result.output


# --- the generated class is the one the register asked for ---------------------------


def test_the_generated_plan_class_has_the_fields_the_register_names():
    """`plan.baml` is the source and `baml_client` is generated from it, so this is the
    only place the register's field list can be checked against what was built."""
    assert set(types.Plan.model_fields) == {"rows", "questions", "confidence", "expressible", "reason"}
    assert set(types.PlanRow.model_fields) == {"if_", "then"}
    assert set(types.Answer.model_fields) == {"question", "answer"}


# --- the prompts themselves ------------------------------------------------------


def request(name, **kwargs):
    """Build the HTTP request BAML would send, without sending it.

    No key, no network, no spend — and it is the only way to find out whether the
    template in `plan.baml` renders at all. `baml-cli generate` checks the syntax of that
    file; it cannot check that the DSL string in `plan.py` arrives in the prompt, or that
    the block guarding the assistant's system prompt disappears when there is none.
    """
    from baml_client.sync_client import b

    body = getattr(b.request, name)(**kwargs).body.json()
    return "\n".join(part["text"] for part in body["system"]), body["messages"][-1]["content"][0]["text"]


def test_the_plan_prompt_carries_the_dsl_and_the_assistants_own_prompt():
    system, user = request(
        "PlanPattern",
        criterion="calls where the caller asked for a person",
        system_prompt="You book trucks. Use bookAppointment.",
        dsl=planning.DSL,
    )

    assert "tool_not_called" in system and "not expressible" in system
    assert "Use bookAppointment." in system
    # The output schema is what makes the answer parseable at all.
    assert "expressible" in system and "if_" in system
    assert user == "calls where the caller asked for a person"


def test_the_assistant_prompt_block_is_gone_when_there_is_no_prompt():
    """A `None` must skip the block, not render an empty quotation for the model to
    puzzle over."""
    system, _ = request("PlanPattern", criterion="asked for a person", system_prompt=None, dsl=planning.DSL)

    assert "system prompt below" not in system
    assert "tool_not_called" in system


def test_the_clarify_prompt_carries_the_plan_the_answers_and_the_dsl():
    system, user = request(
        "ClarifyPattern",
        criterion="calls where the caller asked for a person",
        plan=a_plan(confidence=0.7, questions=["Does the manager count?"]),
        answers=[
            types.Answer(question="Does the manager count?", answer="Yes."),
            types.Answer(question="Does it count when the assistant offers?", answer="No."),
        ],
        dsl=planning.DSL,
    )

    # An answer can add a row the DSL cannot check, and `expressible` is what the wizard
    # opens its spend button on. A clarify that could not see the DSL could only repeat
    # the flag it was handed.
    assert "not expressible" in system
    assert "the caller asks for a person" in user
    assert "0.7" in user
    assert "A: Yes." in user and "A: No." in user
