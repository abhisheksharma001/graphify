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

#: `charged` before the autouse fixture below replaces it. The one test that is about
#: what the provider actually billed has to reach the real function, and by the time it
#: runs the module attribute is a fake.
REAL_CHARGED = planning.charged


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

    def with_options(self, **kwargs):
        """The generated client returns a configured copy of itself; this one is already
        the copy. Its arguments are kept because the client name is one of them, and
        which client was selected is the whole of S-34."""
        self.options = kwargs
        return self

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


#: What the fake provider says every faked call cost. A test that is about the money sets
#: its own; the rest take this one and ignore it.
FAKE_USD = 0.0031


@pytest.fixture(autouse=True)
def priced(monkeypatch):
    """No fake can know what a call it never made was billed for.

    `charged` reads a collector the real client filled in, so every test in this file
    replaces it. Autouse, because a test that forgot would not fail on a wrong price — it
    would fail on `None.usage`, which reads like a bug in the code under test.
    """
    monkeypatch.setattr(planning, "charged", lambda _collector, _model: FAKE_USD)


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


def answer(result):
    """The result line, parsed.

    Stdout carries `ESTIMATE {usd}` before the answer now, so the answer is the last
    non-empty line rather than the whole of stdout. `engine/src/jobs.rs` reads it the same
    way, for the same reason.
    """
    lines = [line for line in result.stdout.splitlines() if line.strip()]
    return json.loads(lines[-1])


def quoted(result):
    """The USD on the `ESTIMATE` line, as a float."""
    line = next(l for l in result.stdout.splitlines() if l.startswith("ESTIMATE "))
    return float(line.removeprefix("ESTIMATE "))


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

    result = run(
        ["plan"],
        '{"criterion": "calls where the caller asked for a person",'
        ' "model": "sonnet", "max_usd": 1}',
    )

    assert result.exit_code == 0
    assert answer(result) == {**fake.reply.model_dump(), "usd": FAKE_USD}
    assert answer(result)["confidence"] == 0.7
    assert len(answer(result)["questions"]) == 2


def test_the_row_condition_is_called_if_underscore_the_whole_way_out(recorder):
    """`if_` in the BAML class, `if_` in the JSON, `if_` in the UI. `if` is a Python
    keyword, so one of those hops would have to rename it, and a field with two names is
    a field somebody reads under the wrong one."""
    recorder(a_plan())

    result = run(["plan"], '{"criterion": "asked for a person", "model": "sonnet", "max_usd": 1}')

    assert answer(result)["rows"][0]["if_"] == "the caller asks for a person"


# --- what the model is told --------------------------------------------------------


def test_the_model_is_shown_the_dsl_it_has_to_plan_against(recorder):
    fake = recorder(a_plan())

    run(["plan"], '{"criterion": "asked for a person", "model": "sonnet", "max_usd": 1}')

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

    run(
        ["plan"],
        '{"criterion": "asked for a person", "system_prompt": "   ",'
        ' "model": "sonnet", "max_usd": 1}',
    )

    assert fake.calls[0][1]["system_prompt"] is None


def test_an_assistant_prompt_is_handed_over_whole(recorder):
    fake = recorder(a_plan())

    run(
        ["plan"],
        json.dumps(
            {
                "criterion": "booked",
                "system_prompt": "You book trucks.\nUse bookAppointment.",
                "model": "sonnet",
                "max_usd": 1,
            }
        ),
    )

    assert fake.calls[0][1]["system_prompt"] == "You book trucks.\nUse bookAppointment."


# --- inputs that are refused, without spending anything ------------------------------


def test_an_empty_criterion_is_refused_before_the_model(never):
    result = run(["plan"], '{"criterion": "   ", "model": "sonnet", "max_usd": 1}')

    assert result.exit_code == 1
    assert "criterion must be a non-empty string" in result.output


def test_a_missing_criterion_is_refused(never):
    result = run(["plan"], "{}")

    assert result.exit_code == 1
    assert "missing criterion" in result.output


def test_a_misspelled_field_is_named_rather_than_dropped(never):
    """`criteria` for `criterion` would otherwise arrive as no criterion at all, and the
    model would answer about some other question with a confidence attached."""
    result = run(
        ["plan"],
        '{"criterion": "asked for a person", "systemprompt": "x",'
        ' "model": "sonnet", "max_usd": 1}',
    )

    assert result.exit_code == 1
    assert "has no field systemprompt" in result.output


def test_stdin_that_is_not_json_exits_one(never):
    result = run(["plan"], "criterion: asked for a person")

    assert result.exit_code == 1
    assert "stdin is not JSON" in result.output


def test_a_db_that_is_not_there_fails_before_the_model(never, tmp_path):
    """`--db` is checked, not read. A path that is wrong is wrong at the first step of
    the wizard rather than three functions later, after the labelling has been paid for."""
    result = run(
        ["plan", "--db", str(tmp_path / "nope.db")],
        '{"criterion": "asked", "model": "sonnet", "max_usd": 1}',
    )

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
            "model": "sonnet", "max_usd": 1,
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
    assert answer(result)["confidence"] == 0.96


def test_clarify_with_no_answers_is_refused(never):
    stdin = json.dumps(
        {
            "criterion": "asked for a person",
            "plan": a_plan().model_dump(),
            "answers": [],
            "model": "sonnet",
            "max_usd": 1,
        }
    )

    result = run(["clarify"], stdin)

    assert result.exit_code == 1
    assert "nothing to revise" in result.output


def test_clarify_refuses_a_plan_that_is_not_one(never):
    """The plan comes back from a UI that got it from here. One that no longer fits the
    class is one this brain did not write, and guessing the missing field is worse."""
    broken = a_plan().model_dump()
    del broken["expressible"]
    stdin = json.dumps(
        {
            "criterion": "asked",
            "plan": broken,
            "answers": [{"question": "q", "answer": "a"}],
            "model": "sonnet",
            "max_usd": 1,
        }
    )

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


# --- what it costs ----------------------------------------------------------------
#
# S-33. Before this every message in the wizard's chat called a model, reported nothing,
# and was counted by no cap — the one place the register left the spec's "no model call
# without a shown cost and an explicit go" broken. The go is still the Send button. These
# are about the other half.


def test_a_plan_reports_what_it_actually_cost(recorder):
    """The acceptance. `jobs.rs` books whatever is in `usd`, so a plan that does not carry
    one is a plan the day's spend never hears about."""
    recorder(a_plan())

    result = run(["plan"], '{"criterion": "asked for a person", "model": "sonnet", "max_usd": 1}')

    assert result.exit_code == 0
    assert answer(result)["usd"] == FAKE_USD


def test_a_clarify_reports_what_it_actually_cost(recorder):
    recorder(a_plan(confidence=0.96, questions=[]))
    stdin = json.dumps(
        {
            "criterion": "asked for a person",
            "plan": a_plan(confidence=0.7, questions=["Does the manager count?"]).model_dump(),
            "answers": [{"question": "Does the manager count?", "answer": "Yes."}],
            "model": "sonnet", "max_usd": 1,
        }
    )

    result = run(["clarify"], stdin)

    assert result.exit_code == 0
    assert answer(result)["usd"] == FAKE_USD


def test_the_price_is_on_stdout_before_the_answer(recorder):
    """`ESTIMATE` first, the answer last. That order is the engine's contract, not a
    presentation choice: `jobs.rs` logs every ESTIMATE line and keeps the last other line
    as the result."""
    recorder(a_plan())

    result = run(["plan"], '{"criterion": "asked for a person", "model": "sonnet", "max_usd": 1}')

    lines = [line for line in result.stdout.splitlines() if line.strip()]
    assert lines[0].startswith("ESTIMATE ")
    # To four places, which is what `ESTIMATE` prints — the line is for a person and for
    # the engine's log, and neither wants nine decimals of a cent.
    assert quoted(result) == pytest.approx(
        planning.plan_usd("asked for a person", None, "sonnet"), abs=1e-4
    )


def test_a_message_over_the_cap_is_refused_before_the_model(never):
    """The half of the rule a function that never parks has to keep on its own. `never`
    raises on any attribute access, so "no model was called" is asserted by the fake."""
    result = run(
        ["plan"], '{"criterion": "asked for a person", "model": "sonnet", "max_usd": 0.0001}'
    )

    assert result.exit_code == 1
    assert "over the $0.0001 cap" in result.output


def test_the_price_is_still_printed_for_a_message_that_is_refused(never):
    """A message turned down for being too expensive is exactly the one whose price is
    worth having in the log."""
    result = run(
        ["plan"], '{"criterion": "asked for a person", "model": "sonnet", "max_usd": 0.0001}'
    )

    assert quoted(result) > 0.0001


def test_a_cap_that_is_not_a_cap_is_refused(never):
    for bad in ("0", "-1", '"1"', "true", "null"):
        result = run(["plan"], '{"criterion": "asked", "model": "sonnet", "max_usd": %s}' % bad)

        assert result.exit_code == 1, bad
        assert "max_usd must be a positive number" in result.output, bad


def test_a_message_with_no_cap_is_refused_by_name(never):
    for args, stdin in (
        (["plan"], '{"criterion": "asked", "model": "sonnet"}'),
        (
            ["clarify"],
            json.dumps(
                {
                    "criterion": "a",
                    "plan": a_plan().model_dump(),
                    "answers": [{"question": "q", "answer": "a"}],
                    "model": "sonnet",
                }
            ),
        ),
    ):
        result = run(args, stdin)

        assert result.exit_code == 1
        assert "is missing max_usd" in result.output


def test_the_ceiling_covers_the_prompt_that_is_actually_rendered():
    """`FIXED_PROMPT_CHARS` is a measurement, and a prompt that grows past it turns the
    ceiling into a guess. Rendered here rather than trusted, the same way `test_label.py`
    holds its own."""
    cases = [
        (
            request(
                "PlanPattern",
                criterion="asked for a person",
                system_prompt="You book trucks.",
                dsl=planning.DSL,
            ),
            len(planning.DSL) + len("asked for a person") + len("You book trucks."),
        ),
        (
            request(
                "PlanPattern", criterion="asked for a person", system_prompt=None, dsl=planning.DSL
            ),
            len(planning.DSL) + len("asked for a person"),
        ),
    ]
    for (system, user), variable in cases:
        fixed = len(system) + len(user) - variable
        assert fixed <= planning.FIXED_PROMPT_CHARS, fixed


def test_the_clarify_ceiling_covers_its_rendered_prompt():
    prior = a_plan(confidence=0.7, questions=["Does the manager count?"])
    given = [types.Answer(question="Does the manager count?", answer="Yes.")]
    system, user = request(
        "ClarifyPattern", criterion="asked", plan=prior, answers=given, dsl=planning.DSL
    )

    variable = (
        len(planning.DSL)
        + len("asked")
        + len(prior.model_dump_json())
        + sum(len(a.question) + len(a.answer) for a in given)
    )
    assert len(system) + len(user) - variable <= planning.FIXED_PROMPT_CHARS


# --- S-34: the model the wizard picked is the model that runs -----------------------
#
# Step 1 of the wizard has a Model select, and until now `plan` and `clarify` were the
# only functions that ignored it. Since S-33 that also meant quoting Sonnet's price for a
# call somebody asked to be Opus, which is worse than quoting nothing.


@pytest.mark.parametrize("named,client", [("opus", "Opus"), ("sonnet", "Sonnet"), ("gpt", "GPT")])
def test_a_plan_runs_on_the_client_the_request_named(recorder, named, client):
    fake = recorder(a_plan())

    result = run(["plan"], '{"criterion": "asked", "model": "%s", "max_usd": 1}' % named)

    assert result.exit_code == 0, result.output
    assert fake.options["client"] == client


@pytest.mark.parametrize("named,client", [("opus", "Opus"), ("sonnet", "Sonnet"), ("gpt", "GPT")])
def test_a_clarify_runs_on_the_client_the_request_named(recorder, named, client):
    fake = recorder(a_plan())
    stdin = json.dumps(
        {
            "criterion": "asked",
            "plan": a_plan().model_dump(),
            "answers": [{"question": "q", "answer": "a"}],
            "model": named,
            "max_usd": 1,
        }
    )

    result = run(["clarify"], stdin)

    assert result.exit_code == 0, result.output
    assert fake.options["client"] == client


def test_the_quoted_price_is_the_named_model_s_rate(recorder):
    """The reason this step exists. Opus is two and a half times Sonnet on input and
    output alike, so a quote that does not move with the picker is a quote for a call
    nobody made."""
    recorder(a_plan())

    prices = {}
    for named in ("sonnet", "opus"):
        result = run(["plan"], '{"criterion": "asked", "model": "%s", "max_usd": 1}' % named)
        prices[named] = quoted(result)

    assert prices["opus"] > prices["sonnet"]
    assert prices["sonnet"] == pytest.approx(planning.plan_usd("asked", None, "sonnet"), abs=1e-4)
    assert prices["opus"] == pytest.approx(planning.plan_usd("asked", None, "opus"), abs=1e-4)


def test_what_is_booked_is_the_named_model_s_rate_too():
    """`charged` is faked everywhere else in this file, so it is called directly here.

    A ceiling at the right model and a charge at the wrong one would still put the wrong
    number in `jobs.cost_usd`, and that is the number the day's spend is built from."""

    class Usage:
        input_tokens = 1_000_000
        output_tokens = 0

    class Last:
        usage = Usage()

    class Collector:
        last = Last()

    assert REAL_CHARGED(Collector(), "sonnet") == pytest.approx(2.00)
    assert REAL_CHARGED(Collector(), "opus") == pytest.approx(5.00)


def test_a_model_nobody_prices_is_refused_before_the_model(never):
    """Refused, not defaulted. A model that falls back to a cheap one on a typo is a
    model whose price nobody chose."""
    result = run(["plan"], '{"criterion": "asked", "model": "gemini", "max_usd": 1}')

    assert result.exit_code == 1
    assert "plan: model must be one of gpt, opus, sonnet" in result.output


def test_a_message_with_no_model_is_refused_by_name(never):
    for args, stdin in (
        (["plan"], '{"criterion": "asked", "max_usd": 1}'),
        (
            ["clarify"],
            json.dumps(
                {
                    "criterion": "a",
                    "plan": a_plan().model_dump(),
                    "answers": [{"question": "q", "answer": "a"}],
                    "max_usd": 1,
                }
            ),
        ),
    ):
        result = run(args, stdin)

        assert result.exit_code == 1
        assert "is missing model" in result.output
