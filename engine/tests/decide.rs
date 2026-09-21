//! The seam, held to the two promises that make it worth having: a question set that cannot
//! be asked is refused before anybody asks it, and answers that do not match the questions
//! are refused before anybody reads them.
//!
//! Every test here is arithmetic and string handling over values built in the test. Nothing
//! opens a socket, reads a key, or touches a file except the last test, which walks
//! `engine/src` to prove the seam still has no caller.

use graphify::decide::{Answer, DecisionModel, Decisions, Fake, Question, Questions};
use std::path::{Path, PathBuf};

fn binary() -> Question {
    Question::Binary {
        instructions: "The caller asked to speak to a person.".into(),
        when_true: "The caller asked, in their own words, for a human.".into(),
        when_false: "Anything else, including the agent offering one.".into(),
    }
}

fn choice() -> Question {
    Question::Choice {
        instructions: "Who ended the call.".into(),
        options: vec![
            ("caller".into(), "The caller hung up.".into()),
            ("agent".into(), "The agent ended it.".into()),
            ("other".into(), "Neither, or it cannot be told.".into()),
        ],
    }
}

fn score() -> Question {
    Question::Score {
        instructions: "How soon this needs a person.".into(),
        levels: vec![
            "can wait".into(),
            "this week".into(),
            "today".into(),
        ],
    }
}

fn one() -> Questions {
    Questions::new(vec![("wants_human", binary())]).unwrap()
}

/// The refusal §8 asks for by name. Nothing else in this file can be trusted if a set that
/// asks nothing is a set.
#[test]
fn an_empty_question_set_cannot_be_built() {
    let err = Questions::new(vec![]).unwrap_err().to_string();
    assert!(err.contains("nothing to decide"), "{err}");
}

#[test]
fn a_question_set_that_can_be_asked_is_built_and_keeps_its_order() {
    let asked = Questions::new(vec![
        ("wants_human", binary()),
        ("who_ended", choice()),
        ("urgency", score()),
    ])
    .unwrap();

    assert_eq!(asked.len(), 3);
    assert!(!asked.is_empty());
    assert_eq!(
        asked.names().collect::<Vec<_>>(),
        vec!["urgency", "wants_human", "who_ended"]
    );
    assert!(asked.get("wants_human").is_some());
    assert!(asked.get("never_asked").is_none());
}

#[test]
fn a_question_with_no_name_or_no_instructions_is_refused() {
    let blank_name = Questions::new(vec![("   ", binary())]).unwrap_err().to_string();
    assert!(blank_name.contains("no name"), "{blank_name}");

    let no_instructions = Questions::new(vec![(
        "wants_human",
        Question::Binary {
            instructions: "  ".into(),
            when_true: "yes".into(),
            when_false: "no".into(),
        },
    )])
    .unwrap_err()
    .to_string();
    assert!(no_instructions.contains("no instructions"), "{no_instructions}");
}

/// A true/false question that says what only one of its answers means is the wording
/// failure `docs/prd-jev.md` §6 is about, and it is cheap to refuse here.
#[test]
fn a_binary_that_does_not_say_what_both_answers_mean_is_refused() {
    let err = Questions::new(vec![(
        "wants_human",
        Question::Binary {
            instructions: "The caller asked for a person.".into(),
            when_true: "They asked.".into(),
            when_false: "   ".into(),
        },
    )])
    .unwrap_err()
    .to_string();
    assert!(err.contains("does not say what both answers mean"), "{err}");
}

#[test]
fn the_same_question_cannot_be_asked_twice() {
    let err = Questions::new(vec![("wants_human", binary()), ("wants_human", binary())])
        .unwrap_err()
        .to_string();
    assert!(err.contains("asked twice"), "{err}");
}

#[test]
fn a_choice_smaller_than_two_or_larger_than_a_byte_is_refused() {
    let one_option = Question::Choice {
        instructions: "Who ended the call.".into(),
        options: vec![("caller".into(), "The caller hung up.".into())],
    };
    let err = Questions::new(vec![("who_ended", one_option)]).unwrap_err().to_string();
    assert!(err.contains("1 options"), "{err}");

    let many = Question::Choice {
        instructions: "Which of these.".into(),
        options: (0..256).map(|i| (format!("o{i}"), format!("c{i}"))).collect(),
    };
    let err = Questions::new(vec![("which", many)]).unwrap_err().to_string();
    assert!(err.contains("256 options"), "{err}");
}

#[test]
fn a_choice_that_offers_the_same_option_twice_is_refused() {
    let repeated = Question::Choice {
        instructions: "Who ended the call.".into(),
        options: vec![
            ("caller".into(), "The caller hung up.".into()),
            ("caller".into(), "Also the caller.".into()),
        ],
    };
    let err = Questions::new(vec![("who_ended", repeated)]).unwrap_err().to_string();
    assert!(err.contains("offers caller twice"), "{err}");
}

#[test]
fn a_score_outside_two_to_ten_levels_is_refused() {
    let flat = Question::Score {
        instructions: "How urgent.".into(),
        levels: vec!["only one".into()],
    };
    let err = Questions::new(vec![("urgency", flat)]).unwrap_err().to_string();
    assert!(err.contains("1 levels"), "{err}");

    let eleven = Question::Score {
        instructions: "How urgent.".into(),
        levels: (0..11).map(|i| format!("level {i}")).collect(),
    };
    let err = Questions::new(vec![("urgency", eleven)]).unwrap_err().to_string();
    assert!(err.contains("11 levels"), "{err}");
}

#[test]
fn answers_that_match_the_questions_are_accepted_and_carry_their_model() {
    let asked = Questions::new(vec![
        ("wants_human", binary()),
        ("who_ended", choice()),
        ("urgency", score()),
    ])
    .unwrap();

    let out = Decisions::checked(
        "fake-1.0.0",
        vec![
            ("wants_human", Answer::Binary(0.91)),
            (
                "who_ended",
                Answer::Choice {
                    option: "caller".into(),
                    confidence: 0.8,
                },
            ),
            ("urgency", Answer::Score(2.0)),
        ],
        &asked,
    )
    .unwrap();

    assert_eq!(out.model(), "fake-1.0.0");
    assert_eq!(out.get("wants_human"), Some(&Answer::Binary(0.91)));
    assert_eq!(out.iter().count(), 3);
}

/// Must-never #5 at the seam. The point is not that the error is nice; it is that there is
/// no other branch — no default, no zero, no first option.
#[test]
fn a_question_that_came_back_unanswered_is_an_error_and_not_a_value() {
    let asked = Questions::new(vec![("wants_human", binary()), ("who_ended", choice())]).unwrap();

    let err = Decisions::checked("fake-1.0.0", vec![("wants_human", Answer::Binary(0.9))], &asked)
        .unwrap_err()
        .to_string();
    assert!(err.contains("who_ended came back unanswered"), "{err}");
}

#[test]
fn an_answer_to_a_question_nobody_asked_is_refused() {
    let err = Decisions::checked("fake-1.0.0", vec![("urgency", Answer::Score(1.0))], &one())
        .unwrap_err()
        .to_string();
    assert!(err.contains("was not asked"), "{err}");
}

#[test]
fn the_same_question_cannot_be_answered_twice() {
    let err = Decisions::checked(
        "fake-1.0.0",
        vec![
            ("wants_human", Answer::Binary(0.9)),
            ("wants_human", Answer::Binary(0.1)),
        ],
        &one(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("answered twice"), "{err}");
}

#[test]
fn an_answer_of_the_wrong_kind_is_refused() {
    let err = Decisions::checked("fake-1.0.0", vec![("wants_human", Answer::Score(1.0))], &one())
        .unwrap_err()
        .to_string();
    assert!(err.contains("is a binary and was answered with a score"), "{err}");
}

#[test]
fn a_number_that_is_not_a_probability_is_refused() {
    for bad in [1.01_f64, -0.0001, f64::NAN, f64::INFINITY] {
        let given = vec![("wants_human", Answer::Binary(bad))];
        let err = Decisions::checked("fake-1.0.0", given, &one())
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a probability"), "{bad}: {err}");
    }

    let asked = Questions::new(vec![("who_ended", choice())]).unwrap();
    let err = Decisions::checked(
        "fake-1.0.0",
        vec![(
            "who_ended",
            Answer::Choice {
                option: "caller".into(),
                confidence: 1.5,
            },
        )],
        &asked,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("not a probability"), "{err}");
}

#[test]
fn a_choice_outside_the_options_and_a_level_outside_the_scale_are_refused() {
    let asked = Questions::new(vec![("who_ended", choice()), ("urgency", score())]).unwrap();

    let err = Decisions::checked(
        "fake-1.0.0",
        vec![
            (
                "who_ended",
                Answer::Choice {
                    option: "supervisor".into(),
                    confidence: 0.9,
                },
            ),
            ("urgency", Answer::Score(1.0)),
        ],
        &asked,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("not one of its options"), "{err}");

    let err = Decisions::checked(
        "fake-1.0.0",
        vec![
            (
                "who_ended",
                Answer::Choice {
                    option: "caller".into(),
                    confidence: 0.9,
                },
            ),
            ("urgency", Answer::Score(3.0)),
        ],
        &asked,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("outside its 0..=2 levels"), "{err}");
}

/// Answers with no model behind them are the shape that makes a pin useless later, so the
/// seam will not carry them at all.
#[test]
fn answers_that_do_not_say_which_model_gave_them_are_refused() {
    let err = Decisions::checked("   ", vec![("wants_human", Answer::Binary(0.9))], &one())
        .unwrap_err()
        .to_string();
    assert!(err.contains("which model"), "{err}");
}

#[tokio::test]
async fn the_fake_answers_from_its_table_and_records_what_it_was_asked() {
    let fake = Fake::new("fake-1.0.0")
        .answering("wants_human", Answer::Binary(0.93))
        .answering("who_ended", Answer::Choice {
            option: "agent".into(),
            confidence: 0.7,
        });
    let asked = Questions::new(vec![("wants_human", binary()), ("who_ended", choice())]).unwrap();

    let out = fake.decide("caller: put me through to someone", &asked).await.unwrap();

    assert_eq!(fake.name(), "fake-1.0.0");
    assert_eq!(out.model(), "fake-1.0.0");
    assert_eq!(out.get("wants_human"), Some(&Answer::Binary(0.93)));
    assert_eq!(fake.seen(), vec!["caller: put me through to someone".to_string()]);
}

/// The fake inherits the seam's refusals rather than restating them: it builds its answers
/// through the same checked constructor, so a table that does not fit the questions fails
/// the same way a provider would.
#[tokio::test]
async fn the_fake_fails_rather_than_guesses_when_it_has_no_answer() {
    let fake = Fake::new("fake-1.0.0").answering("wants_human", Answer::Binary(0.93));
    let asked = Questions::new(vec![("wants_human", binary()), ("who_ended", choice())]).unwrap();

    let err = fake.decide("anything", &asked).await.unwrap_err().to_string();
    assert!(err.contains("asked who_ended, which it has no answer for"), "{err}");

    let wrong_kind = Fake::new("fake-1.0.0").answering("wants_human", Answer::Score(1.0));
    let err = wrong_kind.decide("anything", &one()).await.unwrap_err().to_string();
    assert!(err.contains("answered with a score"), "{err}");

    // The case the two above do not cover, and the one that matters: a substitute of the
    // *right kind* for a missing answer. `Decisions::checked` cannot see it — a probability
    // of 0.0 is a valid probability — so this is the only thing standing between a provider
    // that answered nothing and a confident "no" downstream.
    let empty = Fake::new("fake-1.0.0");
    let err = empty.decide("anything", &one()).await.unwrap_err().to_string();
    assert!(err.contains("asked wants_human, which it has no answer for"), "{err}");
}

/// Why the trait is spelled out with a boxed future instead of `async fn`: so a caller can
/// hold a provider it chose at run time. If this stops compiling the seam has stopped being
/// a seam.
#[tokio::test]
async fn a_provider_can_be_chosen_at_run_time() {
    let providers: Vec<Box<dyn DecisionModel>> = vec![
        Box::new(Fake::new("fake-a").answering("wants_human", Answer::Binary(0.2))),
        Box::new(Fake::new("fake-b").answering("wants_human", Answer::Binary(0.8))),
    ];

    let mut said: Vec<(String, Answer)> = Vec::new();
    for provider in &providers {
        let out = provider.decide("the same transcript", &one()).await.unwrap();
        said.push((out.model().to_string(), out.get("wants_human").unwrap().clone()));
    }

    assert_eq!(
        said,
        vec![
            ("fake-a".to_string(), Answer::Binary(0.2)),
            ("fake-b".to_string(), Answer::Binary(0.8)),
        ]
    );
}

/// The flag, in the only form that cannot be left on by accident: the seam has no caller.
///
/// **Delete this test in the step that wires the first one** (S-73 and after). It is here so
/// that wiring is a deliberate edit to a test that says what it is guarding, rather than a
/// line nobody notices.
#[test]
fn the_seam_has_no_caller_yet() {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("engine/src is readable") {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    assert!(files.len() >= 15, "the walk is not reading engine/src: {files:?}");

    let mut checked = 0;
    for path in files {
        let name = path.strip_prefix(&src).unwrap().to_string_lossy().into_owned();
        if name == "decide.rs" || name == "lib.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("DecisionModel") && !text.contains("decide::"),
            "{name} uses the decision seam. S-71 ships it dark; the step that wires the \
             first caller deletes this test on purpose."
        );
        checked += 1;
    }
    assert!(checked >= 15, "only {checked} files were checked");
}
