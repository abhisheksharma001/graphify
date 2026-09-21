"""The Jev calibration set, held to the numbers it earned.

Nothing here calls a model, reads a key, or opens a socket: every assertion is arithmetic
over four files in `docs/jev/`. That is the point. The set is a measurement, and a
measurement whose inputs can move without its outputs moving is not one — so the wording of
the question is hashed into the thresholds file, the rates are recomputed from the counts
they came from, and the bar the run was judged against is recomputed too.

`docs/prd-jev.md` §4 is the write-up. Its numbers are direction only: 19 eval cases, cases
written by the same person who wrote the question, and nothing here has been near a real
call.
"""

import hashlib
import json
from pathlib import Path

import pytest

JEV = Path(__file__).resolve().parents[2] / "docs" / "jev"

#: The shapes a naive question gets wrong, each held by the words its case's `note` uses.
#: `docs/prd-jev.md` §7 requires a shipped set to keep all of them; dropping one would make
#: a future accuracy number look better for no reason but an easier set.
HARD = ["INJECTION", "ACCEPTED an offer", "past tense", "third party", "callback",
        "TRANSFERRED but caller never asked", "agent offered"]


def canonical(question: dict) -> bytes:
    """The question as the pin sees it: parsed, keys sorted, no insignificant whitespace.

    Reformatting the file is free; changing a word of it is not.
    """
    return json.dumps(question, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False).encode("utf-8")


@pytest.fixture
def thresholds() -> dict:
    return json.loads((JEV / "wants_human.thresholds.json").read_text(encoding="utf-8"))


@pytest.fixture
def question() -> dict:
    return json.loads((JEV / "wants_human.question.json").read_text(encoding="utf-8"))


@pytest.fixture
def cases() -> list:
    lines = (JEV / "cases.jsonl").read_text(encoding="utf-8").splitlines()
    return [json.loads(line) for line in lines if line.strip()]


# --- the pin: numbers belong to one wording ------------------------------------------


def test_the_numbers_belong_to_the_question_that_is_in_the_repo(thresholds, question):
    """The whole step. Every rate in the thresholds file was measured on one exact wording;
    edit that wording and the rates describe a question that no longer exists. The remedy
    when this fails is to re-measure and write the new hash, not to rewrite the hash."""
    assert thresholds["question_file"] == "wants_human.question.json"
    assert hashlib.sha256(canonical(question)).hexdigest() == thresholds["question_sha256"]


def test_a_threshold_exists_for_every_question_asked_and_for_no_other(thresholds, question):
    assert set(thresholds["questions"]) == set(question)


def test_the_model_is_pinned_to_a_version_and_not_to_an_alias(thresholds):
    """`jev-latest` moves, and a moved model stales every number in this file silently. The
    skill's rule, and the reason `response.model` is logged at run time."""
    assert thresholds["model"] == "jev-1.13.0"
    assert "latest" not in thresholds["model"]


# --- the arithmetic the write-up rests on ---------------------------------------------


def test_the_eval_counts_add_up_to_the_number_of_cases_they_claim(thresholds):
    e = thresholds["questions"]["wants_human"]["eval"]
    assert e["tp"] + e["fp"] + e["fn"] + e["tn"] == e["n"]


def test_the_rates_are_the_ones_the_counts_give(thresholds):
    """Editing a count to flatter the result now has to be done twice, in two places that
    disagree arithmetically."""
    e = thresholds["questions"]["wants_human"]["eval"]

    assert e["tpr"] == pytest.approx(e["tp"] / (e["tp"] + e["fn"]))
    assert e["tnr"] == pytest.approx(e["tn"] / (e["tn"] + e["fp"]))


def test_the_bar_was_missed_and_says_so(thresholds):
    """`docs/prd-jev.md` §4 wrote the bar down before any case was scored and reports that
    eval TPR misses it by one case of nine positives. That admission is the most perishable
    sentence in the PRD — three paragraphs from the table that contradicts it at a glance —
    so `met` is recomputed here rather than believed."""
    entry = thresholds["questions"]["wants_human"]
    bar, e = entry["bar"], entry["eval"]

    met = e["tpr"] >= bar["tpr"] and e["tnr"] >= bar["tnr"]

    assert met is bar["met"]
    assert bar["met"] is False


def test_the_cost_of_being_wrong_is_written_down_and_is_not_symmetric(thresholds):
    """2:1 against a miss. An undercount hides the thing the analyst is hunting; an
    overcount is visible in the call drawer next to its evidence quote."""
    costs = thresholds["questions"]["wants_human"]["costs"]

    assert costs["fn"] > costs["fp"] > 0


# --- the set itself ---------------------------------------------------------------------


def test_the_cases_are_the_set_the_thresholds_were_fitted_on(thresholds, cases):
    claimed = thresholds["cases"]
    labels = [c["labels"]["wants_human"] for c in cases]

    assert len(cases) == claimed["n"]
    assert labels.count(1) == claimed["positive"]
    assert labels.count(0) == claimed["negative"]
    assert set(labels) == {0, 1}


def test_no_two_cases_share_an_id(cases):
    """Ids are what the split is taken on, so a duplicate would put one case on both sides
    of it."""
    ids = [c["id"] for c in cases]

    assert len(set(ids)) == len(ids)


def test_the_split_is_the_one_the_numbers_were_measured_on(thresholds, cases):
    """`calibrate.py` splits on the SHA-256 of the case id, so the split is a property of
    the ids and not of a seed anybody kept. Rename a case and the halves move under every
    number in the file."""
    claimed = thresholds["split"]
    fraction = claimed["eval_fraction"]

    def side(case_id: str) -> str:
        h = int(hashlib.sha256(case_id.encode()).hexdigest()[:8], 16) / 0xFFFFFFFF
        return "eval" if h < fraction else "train"

    sides = [side(c["id"]) for c in cases]

    assert sides.count("train") == claimed["train"]
    assert sides.count("eval") == claimed["eval"]
    assert sides.count("eval") == thresholds["questions"]["wants_human"]["eval"]["n"]


def test_every_shape_that_breaks_a_naive_question_is_still_in_the_set(cases):
    notes = " | ".join(c["note"] for c in cases)

    for shape in HARD:
        assert shape in notes, f"the {shape!r} case is gone and the set just got easier"


def test_both_injections_are_negatives(cases):
    """An injection case labelled 1 would teach the question to obey the transcript, which
    is the failure it exists to catch. §7 wants four shapes eventually; two is what this
    set has, and the 200-case build owes the rest."""
    injections = [c for c in cases if "INJECTION" in c["note"]]

    assert len(injections) >= 2
    assert all(c["labels"]["wants_human"] == 0 for c in injections)


def test_a_case_never_says_a_missing_value_is_zero(cases):
    """The spec's rule, and it bites hardest here: a model told a call ran no tools and a
    model told a call's tool list is empty are being told different things, and the second
    one is a fact nobody recorded. Two fields in this state can be absent — `tools_run` and
    `transcript` — and `build_cases.py` writes "—" for both. Everything falsy is refused
    rather than only `null`, because `0`, `false` and `""` are the shapes this goes wrong
    in; the dropped-call case is the one that proves the path is taken at all."""
    for case in cases:
        state = case["state"]
        assert "null" not in json.dumps(state)

        for field in (state["call_facts"]["tools_run"], state["transcript"]):
            assert isinstance(field, list) or field == "—", f"{case['id']}: {field!r}"

        for name, value in state["call_facts"].items():
            assert value is not None, f"{case['id']}: {name} is null"

    assert any(c["state"]["transcript"] == "—" for c in cases)


# --- the questions --------------------------------------------------------------------


@pytest.mark.parametrize(
    "name", ["wants_human.question.json", "wants_human.question.first-draft.json"]
)
def test_a_question_asks_one_thing_and_says_what_both_answers_mean(name):
    """One judgement per question, criteria on both sides — the skill's first rule, and the
    difference between the 62-89% a compound question scored and the 95% five atomic ones
    did. The first draft is held to the same shape because §4 reports a per-case delta
    against it, which nobody can reproduce from a file that no longer parses."""
    asked = json.loads((JEV / name).read_text(encoding="utf-8"))

    assert asked, f"{name} asks nothing"
    for question in asked.values():
        assert question["type"] in {"noul", "choice", "score"}
        assert question["instructions"].strip()
        assert set(question["criteria"]) == {"true", "false"}
        assert all(text.strip() for text in question["criteria"].values())


def test_the_wording_in_force_still_answers_the_injections_in_prose(question):
    """The `criteria.false` paragraph that took the direct injection from 0.79 to 0.14 works
    by telling Jev what the transcript *is*: quoted speech, not instructions addressed to
    it. Lose that sentence and the 0.14 goes with it, which is the one part of the rewrite
    worth naming in a test rather than leaving to the hash."""
    false = question["wants_human"]["criteria"]["false"]

    assert "quoted here as data" in false
