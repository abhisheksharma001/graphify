//! The seam a decision provider plugs into, and the only vocabulary the rest of the engine
//! will ever need to learn for one.
//!
//! A decision here is narrow on purpose: a question about some text whose answer comes from
//! a closed set — true or false, one of a named list, one of an ordered list of levels —
//! together with how sure the answerer was. Anything that writes prose is not a decision and
//! does not belong behind this trait; that work stays with the brain.
//!
//! Two of the types below have exactly one constructor each, and both of those constructors
//! can fail. That is the whole design. `Questions::new` refuses a set that cannot be asked,
//! so no adapter has to remember to check; `Decisions::checked` refuses answers that do not
//! match the questions that were asked, so a provider which quietly starts returning
//! something else becomes a loud failure at the boundary instead of a wrong number three
//! screens later. An adapter written against this seam inherits every one of those refusals
//! without repeating a line of them.
//!
//! Nothing in this file reaches the network, reads a key, opens a file, or names a vendor.
//! That last one is D-13 and it is easy to get wrong here: every provider has its own word
//! for "true or false with a probability attached", and translating it into `Binary` is the
//! adapter's job, in the adapter's own file. The only implementation this file ships is
//! `Fake`.

use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

/// A `Choice` needs at least two options to be a choice at all, and no provider worth
/// plugging in here answers from a set larger than a byte.
const OPTIONS: std::ops::RangeInclusive<usize> = 2..=255;

/// A `Score` is an ordered scale a person has to be able to read off. Past ten levels the
/// difference between two of them stops being a judgement anyone can define in a sentence.
const LEVELS: std::ops::RangeInclusive<usize> = 2..=10;

/// One question, with the criteria that decide it. The criteria are part of the question,
/// not documentation of it: the wording is what gets measured, and a set of numbers belongs
/// to the exact text that earned it (S-69).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    /// True or false, answered with the probability that it is true.
    Binary {
        instructions: String,
        when_true: String,
        when_false: String,
    },
    /// One option out of a named set, each with the criteria that select it.
    Choice {
        instructions: String,
        options: Vec<(String, String)>,
    },
    /// One level out of an ordered scale, lowest first.
    Score {
        instructions: String,
        levels: Vec<String>,
    },
}

impl Question {
    /// What this kind of question is called in a failure message.
    fn kind(&self) -> &'static str {
        match self {
            Question::Binary { .. } => "binary",
            Question::Choice { .. } => "choice",
            Question::Score { .. } => "score",
        }
    }

    fn instructions(&self) -> &str {
        match self {
            Question::Binary { instructions, .. }
            | Question::Choice { instructions, .. }
            | Question::Score { instructions, .. } => instructions,
        }
    }
}

/// One answer. `Binary` and `Choice` carry how sure the answerer was; a `Score` carries the
/// level itself, which may fall between two of them.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// The probability that the question is true, 0.0..=1.0. Not a verdict: the cut that
    /// turns it into one is fitted per question, and 0.5 is a knife edge.
    Binary(f64),
    /// The option chosen, and the probability it is the right one.
    Choice { option: String, confidence: f64 },
    /// The level, as an index into the question's `levels`, lowest first.
    Score(f64),
}

impl Answer {
    fn kind(&self) -> &'static str {
        match self {
            Answer::Binary(_) => "binary",
            Answer::Choice { .. } => "choice",
            Answer::Score(_) => "score",
        }
    }
}

/// The questions asked in one go, by name. The only constructor validates, so holding one of
/// these is proof the set can be asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Questions(BTreeMap<String, Question>);

impl Questions {
    /// Build a question set, or say why it cannot be asked.
    ///
    /// Refused: an empty set, a blank name, blank instructions, blank criteria, a name asked
    /// twice, a choice with fewer than two or more than 255 options, a choice whose options
    /// repeat, and a score with fewer than two or more than ten levels.
    pub fn new(asked: Vec<(&str, Question)>) -> Result<Self> {
        if asked.is_empty() {
            bail!("no questions were asked, so there is nothing to decide");
        }

        let mut out: BTreeMap<String, Question> = BTreeMap::new();
        for (name, question) in asked {
            if name.trim().is_empty() {
                bail!("a question has no name");
            }
            if question.instructions().trim().is_empty() {
                bail!("question {name} has no instructions");
            }
            check(name, &question)?;
            if out.insert(name.to_string(), question).is_some() {
                bail!("question {name} was asked twice");
            }
        }
        Ok(Questions(out))
    }

    /// The question asked under this name, if it was asked.
    pub fn get(&self, name: &str) -> Option<&Question> {
        self.0.get(name)
    }

    /// Every name asked, in a stable order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// How many questions are in the set.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Never true: an empty set cannot be built. Here so that `len` reads normally.
    pub fn is_empty(&self) -> bool {
        false
    }
}

/// The per-kind half of `Questions::new`, kept apart so the shared checks above stay one
/// short list and this stays another.
fn check(name: &str, question: &Question) -> Result<()> {
    match question {
        Question::Binary {
            when_true,
            when_false,
            ..
        } => {
            if when_true.trim().is_empty() || when_false.trim().is_empty() {
                bail!("question {name} does not say what both answers mean");
            }
        }
        Question::Choice { options, .. } => {
            if !OPTIONS.contains(&options.len()) {
                bail!(
                    "question {name} offers {} options; a choice takes {}..={}",
                    options.len(),
                    OPTIONS.start(),
                    OPTIONS.end()
                );
            }
            let mut seen: Vec<&str> = Vec::with_capacity(options.len());
            for (option, criteria) in options {
                if option.trim().is_empty() || criteria.trim().is_empty() {
                    bail!("question {name} has an option with no name or no criteria");
                }
                if seen.contains(&option.as_str()) {
                    bail!("question {name} offers {option} twice");
                }
                seen.push(option);
            }
        }
        Question::Score { levels, .. } => {
            if !LEVELS.contains(&levels.len()) {
                bail!(
                    "question {name} has {} levels; a score takes {}..={}",
                    levels.len(),
                    LEVELS.start(),
                    LEVELS.end()
                );
            }
            if levels.iter().any(|l| l.trim().is_empty()) {
                bail!("question {name} has a level with no description");
            }
        }
    }
    Ok(())
}

/// What a decision provider gave back, checked against what it was asked. The only
/// constructor validates, so holding one of these is proof every question has an answer of
/// the right shape — which is why nothing downstream needs a default for a missing one.
#[derive(Debug, Clone, PartialEq)]
pub struct Decisions {
    model: String,
    answers: BTreeMap<String, Answer>,
}

impl Decisions {
    /// Build the answers to `asked`, or say why they are not answers to it.
    ///
    /// Refused: a blank model, an answer to a question nobody asked, a question left
    /// unanswered, an answer answered twice, an answer of the wrong kind, a probability
    /// outside 0.0..=1.0, a choice that is not one of that question's options, and a level
    /// outside that question's scale.
    ///
    /// A question with no answer is an error here and nowhere else. It is never 0.0, never
    /// false, and never the first option: a provider that answers four questions out of five
    /// is exactly the case where a default would read as a result.
    pub fn checked(model: &str, given: Vec<(&str, Answer)>, asked: &Questions) -> Result<Self> {
        if model.trim().is_empty() {
            bail!("the answers do not say which model gave them");
        }

        let mut answers: BTreeMap<String, Answer> = BTreeMap::new();
        for (name, answer) in given {
            let Some(question) = asked.get(name) else {
                bail!("an answer came back for {name}, which was not asked");
            };
            fits(name, question, &answer)?;
            if answers.insert(name.to_string(), answer).is_some() {
                bail!("question {name} was answered twice");
            }
        }

        for name in asked.names() {
            if !answers.contains_key(name) {
                bail!("question {name} came back unanswered");
            }
        }

        Ok(Decisions {
            model: model.to_string(),
            answers,
        })
    }

    /// Which model answered. Carried with the answers rather than assumed, so a pinned
    /// version that has moved underneath us shows up at the boundary and not at the next
    /// recalibration.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The answer to a question that was asked. `None` only for a name that was not in the
    /// set — every name that was, has one.
    pub fn get(&self, name: &str) -> Option<&Answer> {
        self.answers.get(name)
    }

    /// Every answer, by name, in the same order as `Questions::names`.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Answer)> {
        self.answers.iter().map(|(k, v)| (k.as_str(), v))
    }
}

/// Whether one answer can be an answer to one question.
fn fits(name: &str, question: &Question, answer: &Answer) -> Result<()> {
    match (question, answer) {
        (Question::Binary { .. }, Answer::Binary(p)) => probability(name, *p),
        (Question::Choice { options, .. }, Answer::Choice { option, confidence }) => {
            if !options.iter().any(|(o, _)| o == option) {
                bail!("question {name} was answered {option}, which is not one of its options");
            }
            probability(name, *confidence)
        }
        (Question::Score { levels, .. }, Answer::Score(level)) => {
            let top = (levels.len() - 1) as f64;
            if !level.is_finite() || *level < 0.0 || *level > top {
                bail!("question {name} was answered {level}, outside its 0..={top} levels");
            }
            Ok(())
        }
        _ => bail!(
            "question {name} is a {} and was answered with a {}",
            question.kind(),
            answer.kind()
        ),
    }
}

fn probability(name: &str, p: f64) -> Result<()> {
    if !p.is_finite() || !(0.0..=1.0).contains(&p) {
        bail!("question {name} came back with {p}, which is not a probability");
    }
    Ok(())
}

/// The future a `DecisionModel` hands back. Spelled out rather than written `async fn`
/// because an `async fn` in a trait is not `dyn`-compatible on this toolchain (rustc 1.98,
/// E0038), and the point of the trait is choosing the provider at run time.
pub type Answered<'a> = Pin<Box<dyn Future<Output = Result<Decisions>> + Send + 'a>>;

/// Something that can answer a closed question about some text.
///
/// The `state` is whatever the caller has already worked out — counts, dates and arithmetic
/// belong in code, not in the question — and the implementation's job is only the semantic
/// remainder.
pub trait DecisionModel: Send + Sync {
    /// What to write in a log or a ledger beside what this cost. Not a secret, and never a
    /// key: an implementation that needs one holds it somewhere this name does not reach.
    fn name(&self) -> &str;

    /// Answer every question in `asked` about `state`, or fail. Partial answers are not a
    /// result: `Decisions::checked` is the only way to build the return value.
    fn decide<'a>(&'a self, state: &'a str, asked: &'a Questions) -> Answered<'a>;
}

/// A `DecisionModel` that answers from a table a test wrote, so a test never needs a
/// network, a key, or a clock. It records every state it was asked about, and it fails
/// rather than guesses when it is asked something nobody programmed.
#[derive(Debug)]
pub struct Fake {
    model: String,
    answers: BTreeMap<String, Answer>,
    seen: Mutex<Vec<String>>,
}

impl Fake {
    /// A fake that reports itself as `model`.
    pub fn new(model: &str) -> Self {
        Fake {
            model: model.to_string(),
            answers: BTreeMap::new(),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Programme the answer this fake gives for one question name.
    pub fn answering(mut self, name: &str, answer: Answer) -> Self {
        self.answers.insert(name.to_string(), answer);
        self
    }

    /// Every state this fake has been asked about, oldest first.
    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("the fake's log is not poisoned").clone()
    }
}

impl DecisionModel for Fake {
    fn name(&self) -> &str {
        &self.model
    }

    fn decide<'a>(&'a self, state: &'a str, asked: &'a Questions) -> Answered<'a> {
        Box::pin(async move {
            self.seen
                .lock()
                .expect("the fake's log is not poisoned")
                .push(state.to_string());

            let mut given: Vec<(&str, Answer)> = Vec::with_capacity(asked.len());
            for name in asked.names() {
                match self.answers.get(name) {
                    Some(answer) => given.push((name, answer.clone())),
                    None => bail!("the fake was asked {name}, which it has no answer for"),
                }
            }
            Decisions::checked(&self.model, given, asked)
        })
    }
}
