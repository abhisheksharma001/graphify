//! TypeSafe's Jev, behind the decision seam.
//!
//! This is the only file in the tree that knows this vendor exists. The base URL, the
//! pinned model, the price, the wire shapes and the word `noul` all stop here; everything
//! upstream speaks `decide.rs`'s vocabulary and would not change if the provider did
//! (D-13, the same rule that keeps Vapi's JSON inside `vapi.rs`).
//!
//! It is also the one file in the tree allowed to send something that is not a GET, and it
//! sends exactly one thing: a POST to `/v1/systemone`. `engine/tests/outbound.rs` names it
//! on `DECISION_CONNECTORS` and holds it to that over the whole source text (A-1, approved
//! 2026-09-21). A decision provider is allowed to POST because it holds none of our data
//! and owns nothing we could damage — not because a POST is safe.
//!
//! The key is the install's, stored under `typesafe` beside the model keys and read back
//! through `Secrets::get` (S-74). A `Jev` cannot be built without one, so there is no path
//! from here to the network that does not carry a key.
//!
//! Source: `docs/prd-jev.md` §13, checked 2026-09-21.

use crate::db::Db;
use crate::decide::{Answer, Answered, DecisionModel, Decisions, Question, Questions};
use crate::secrets::Secrets;
use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};
use std::time::Duration;

pub const DEFAULT_BASE: &str = "https://api.typesafe.ai";

/// The name the key is stored under, install-wide. `engine/src/secrets.rs` owns the list
/// that name is on; this is the one place that asks for it.
pub const SECRET: &str = "typesafe";

/// Pinned, never `jev-latest`, which moves. A set of thresholds belongs to the exact model
/// that earned it, so the version is part of the request and is read back off the reply.
pub const MODEL: &str = "jev-1.13.0";

/// Output is free; input is what is billed. docs.typesafe.ai/models, checked 2026-09-21.
const USD_PER_M_INPUT: f64 = 0.042;

/// The vendor's limits are 32k tokens of state inside a 64k-token request. A token never
/// spans less than one byte, so a byte count is an upper bound on a token count and these
/// caps hold without a tokenizer — with room left over for whatever the vendor counts that
/// we cannot see from here.
const STATE_MAX: usize = 24_000;
const BODY_MAX: usize = 48_000;

/// A near-64k request has been seen to stall rather than fail, so the client always has a
/// clock on it.
const TIMEOUT: Duration = Duration::from_secs(30);

/// How much of a provider's complaint is worth repeating in an error.
const DETAIL: usize = 500;

/// What one call cost. Computed here because the price is a fact about this vendor, and
/// this is the file that is allowed to know it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub usd: f64,
}

/// A decision provider that asks Jev.
///
/// Deliberately not `Debug`: it holds a key, and the cheapest way to leak one is a struct
/// that prints itself into a log line somebody added while debugging something else.
pub struct Jev {
    base: String,
    key: String,
    timeout: Duration,
}

impl Jev {
    /// The real provider, with the install's key.
    ///
    /// Read through `Secrets::get`, so `TYPESAFE_API_KEY` overrides the store exactly the
    /// way it does for every other key. With neither, this fails and says where to put one.
    pub fn stored(db: &Db, secrets: &Secrets) -> Result<Self> {
        let Some(key) = secrets.get(db, None, SECRET)? else {
            bail!("no TypeSafe key is set: add one in Settings, or set TYPESAFE_API_KEY");
        };
        Jev::new(key.expose())
    }

    /// The real provider.
    pub fn new(key: &str) -> Result<Self> {
        Jev::at(DEFAULT_BASE, key, TIMEOUT)
    }

    /// The same provider against an explicit base and clock, so a test can point at a mock
    /// and not wait thirty seconds to watch a timeout work.
    ///
    /// Refuses a blank key rather than building a client that would send an empty bearer
    /// token. This file is the only one allowed to POST (A-1) and this is the only way to
    /// build the thing that does it, so a request to the decision provider without a key
    /// is not expressible anywhere in the tree — not a rule somebody has to remember.
    pub fn at(base: &str, key: &str, timeout: Duration) -> Result<Self> {
        let key = key.trim();
        if key.is_empty() {
            bail!("a TypeSafe key is required: nothing can be asked without one");
        }
        Ok(Jev {
            base: base.trim_end_matches('/').to_string(),
            key: key.to_string(),
            timeout,
        })
    }

    /// Ask every question in `asked` about `state`, and say what it cost.
    ///
    /// The trait's `decide` drops the `Usage`; this is the way in for the caller that has
    /// to book it. Both fail rather than return a partial answer: a provider that answers
    /// four questions out of five is exactly the case where a default reads as a result.
    pub async fn ask(&self, state: &str, asked: &Questions) -> Result<(Decisions, Usage)> {
        let body = body(state, asked)?;

        let http = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .context("the decision client could not be built")?;

        let sent = http
            .post(format!("{}/v1/systemone", self.base))
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .await
            .context("the decision provider could not be reached")?;

        let status = sent.status();
        let text = sent
            .text()
            .await
            .context("the decision provider's reply could not be read")?;

        if !status.is_success() {
            bail!(
                "the decision provider answered {}: {}",
                status.as_u16(),
                clip(&text)
            );
        }

        let reply: Value = serde_json::from_str(&text).with_context(|| {
            format!("the decision provider's reply is not JSON: {}", clip(&text))
        })?;

        read(&reply, asked)
    }
}

impl DecisionModel for Jev {
    /// The pinned model id — what goes in a ledger beside what a call cost. Never the key.
    fn name(&self) -> &str {
        MODEL
    }

    fn decide<'a>(&'a self, state: &'a str, asked: &'a Questions) -> Answered<'a> {
        Box::pin(async move { self.ask(state, asked).await.map(|(answers, _)| answers) })
    }
}

/// The request, or why it is too big to send. Nothing leaves the process until this
/// returns: a size that the provider would refuse is our mistake, not a round trip.
fn body(state: &str, asked: &Questions) -> Result<Value> {
    if state.len() > STATE_MAX {
        bail!(
            "the state is {} bytes, over the {STATE_MAX}-byte cap by {}",
            state.len(),
            state.len() - STATE_MAX
        );
    }

    let mut questions = Map::new();
    for name in asked.names() {
        let question = asked.get(name).expect("a name from the set is in the set");
        questions.insert(name.to_string(), wire(question));
    }
    let body = json!({ "state": state, "model": MODEL, "questions": questions });

    let size = serde_json::to_vec(&body)
        .context("the request could not be measured")?
        .len();
    if size > BODY_MAX {
        bail!(
            "the request is {size} bytes, over the {BODY_MAX}-byte cap by {}",
            size - BODY_MAX
        );
    }
    Ok(body)
}

/// One question in the vendor's shape. `noul` is their word for a true-or-false question
/// answered with a probability; it is `Question::Binary` everywhere else in this program.
fn wire(question: &Question) -> Value {
    match question {
        Question::Binary {
            instructions,
            when_true,
            when_false,
        } => json!({
            "type": "noul",
            "instructions": instructions,
            "criteria": { "true": when_true, "false": when_false },
        }),
        Question::Choice {
            instructions,
            options,
        } => {
            let criteria: Map<String, Value> = options
                .iter()
                .map(|(option, criteria)| (option.clone(), Value::String(criteria.clone())))
                .collect();
            json!({ "type": "choice", "instructions": instructions, "criteria": criteria })
        }
        Question::Score {
            instructions,
            levels,
        } => json!({ "type": "score", "instructions": instructions, "criteria": levels }),
    }
}

/// The reply, read back into the seam's vocabulary and checked against what was asked.
///
/// The model is taken from the *reply*, not from `MODEL`: a pin that has moved underneath
/// us is then visible at the boundary and in the ledger, instead of at the next
/// recalibration when the numbers no longer mean anything.
fn read(reply: &Value, asked: &Questions) -> Result<(Decisions, Usage)> {
    let Some(model) = reply.get("model").and_then(Value::as_str) else {
        bail!("the decision provider's reply does not say which model answered");
    };
    let Some(answers) = reply.get("answers").and_then(Value::as_object) else {
        bail!("the decision provider's reply carries no answers");
    };

    // An answer to a question nobody asked is the same drift signal as an answer in the
    // wrong shape, and `Decisions::checked` would refuse it — but only if it is handed one.
    // Reading past it here would make that refusal unreachable from the only adapter that
    // can trigger it.
    for name in answers.keys() {
        if asked.get(name).is_none() {
            bail!("the decision provider answered {name}, which was not asked");
        }
    }

    let mut given: Vec<(&str, Answer)> = Vec::with_capacity(asked.len());
    for name in asked.names() {
        let Some(answer) = answers.get(name) else {
            bail!("the decision provider did not answer {name}");
        };
        let question = asked.get(name).expect("a name from the set is in the set");
        given.push((name, one(name, question, answer)?));
    }

    let usage = usage(reply)?;
    Ok((Decisions::checked(model, given, asked)?, usage))
}

/// One answer, read as the kind its question was asked in. A provider that has started
/// answering a choice with a probability fails here by name, rather than by shape three
/// screens later.
fn one(name: &str, question: &Question, answer: &Value) -> Result<Answer> {
    match question {
        Question::Binary { .. } => Ok(Answer::Binary(number(name, answer, "noul")?)),
        Question::Choice { .. } => {
            let Some(option) = answer.get("choice").and_then(Value::as_str) else {
                bail!("the answer to {name} does not say which option was chosen");
            };
            Ok(Answer::Choice {
                option: option.to_string(),
                confidence: number(name, answer, "confidence")?,
            })
        }
        Question::Score { .. } => Ok(Answer::Score(number(name, answer, "score")?)),
    }
}

/// A number the reply was supposed to carry. Absent is an error and never a zero: a
/// missing probability that reads as 0.0 is a confident "no" nobody said.
fn number(name: &str, answer: &Value, field: &str) -> Result<f64> {
    match answer.get(field).and_then(Value::as_f64) {
        Some(n) => Ok(n),
        None => bail!("the answer to {name} carries no {field}, and a missing answer is not a 0"),
    }
}

/// What the call cost, or an error. A reply with no usage is not a free call; it is a call
/// whose price we do not know, and booking an unknown as nothing is how a cap stops working.
fn usage(reply: &Value) -> Result<Usage> {
    let tokens = reply
        .get("usage")
        .and_then(|usage| usage.get("input_tokens"))
        .and_then(Value::as_u64);
    let Some(input_tokens) = tokens else {
        bail!("the decision provider's reply does not say what the call cost");
    };
    Ok(Usage {
        input_tokens,
        usd: input_tokens as f64 * USD_PER_M_INPUT / 1_000_000.0,
    })
}

/// Enough of a provider's complaint to act on, and not a whole page of it in one log line.
fn clip(text: &str) -> String {
    let mut out: String = text.chars().take(DETAIL).collect();
    if text.chars().nth(DETAIL).is_some() {
        out.push('…');
    }
    out
}
