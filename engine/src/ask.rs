//! Pricing one free-form question, and choosing what it is allowed to carry.
//!
//! Every other model call in graphify is quoted by the brain: the engine spawns it, the
//! brain prints `ESTIMATE`, the child parks with its stdin open, and a click sends `GO`.
//! That shape cannot answer this step's acceptance — a person who reads a price and walks
//! away must leave nothing behind — because by the time the brain has quoted, a `jobs` row
//! exists and a Python interpreter is parked against the four the engine allows. Four
//! questions read and abandoned would wedge the wizard.
//!
//! So the quote happens here, on the request, with no subprocess and no row. Which means
//! the engine has to price a model call, and pricing needs rates, and the rates live in
//! `brain/src/graphify_brain/cost.py` where a person reads them off a vendor's page. The
//! table below is a mirror of that one. It is not a second source of truth:
//! `engine/tests/ask.rs` parses `cost.py` and fails if the two disagree, the same way
//! `brain/tests/test_cost.py` parses `clients.baml`. One place to edit, and drift is a
//! failing build rather than a wrong number on a button.
//!
//! The other half of this module is what goes in the context. The statistics describe the
//! whole selection and go in whole. The transcripts are a sample, and the cap is what
//! decides how big it is: shortest first until the tokens run out, at most twenty. Both
//! are counted before anything is sent, and every count here errs high — bytes where the
//! brain counts characters, a flat allowance for the fact line above each transcript —
//! so the engine's figure is the ceiling the brain's own check comes in under.

use crate::db::Db;
use crate::queries::{self, Filters};
use serde::Serialize;

/// Characters per token. `graphify_brain.label.CHARS_PER_TOKEN`, and over-counting for the
/// same reason: an estimate that guards a cap has to err high.
pub const CHARS_PER_TOKEN: usize = 3;

/// The register's ceiling on one question's context, over the statistics, the question and
/// every transcript together. `graphify_brain.ask.MAX_CONTEXT_TOKENS`.
pub const MAX_CONTEXT_TOKENS: usize = 60_000;

/// How many transcripts one question may carry. `graphify_brain.ask.MAX_CALLS`.
pub const MAX_CALLS: usize = 20;

/// The `max_tokens` BAML sends, so the output half of the price is a bound rather than a
/// guess. `graphify_brain.label.MAX_OUTPUT_TOKENS`.
pub const MAX_ANSWER_TOKENS: usize = 4_096;

/// The prompt around it all. `graphify_brain.ask.FIXED_PROMPT_CHARS`.
pub const FIXED_PROMPT_CHARS: usize = 2_600;

/// Allowed per call for the line of recorded facts above its transcript — how long it ran,
/// why it ended, which tools failed. The engine does not build that line, the brain does,
/// so this is an allowance rather than a measurement, and it is set to about three times
/// what a real one runs to.
///
/// The size is a trade, not a safety margin to be maximised. Too small and the engine
/// quotes a context the brain then refuses as over the cap, which is a question that
/// cannot be asked. Too large and every quote is visibly higher than what the brain says
/// the same question costs, which teaches people to stop reading the number on the button.
pub const FACTS_CHARS: usize = 400;

/// Longest question. Not about money — the cap is that — but about what a question is. A
/// pasted document in this box is a document, and it would crowd out the calls it is
/// asking about.
pub const MAX_QUESTION_CHARS: usize = 2_000;

/// One model's published rate in USD per million tokens. Mirrors `graphify_brain.cost`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rate {
    pub provider: &'static str,
    pub model: &'static str,
    pub usd_in: f64,
    pub usd_out: f64,
}

/// A million, named, because the rates are published per million tokens.
const PER: f64 = 1_000_000.0;

/// Keyed by the client nickname, exactly as `cost.PRICES` is — that is the word a job
/// records and the word the browser sends.
pub const PRICES: &[(&str, Rate)] = &[
    (
        "opus",
        Rate {
            provider: "anthropic",
            model: "claude-opus-5",
            usd_in: 5.00,
            usd_out: 25.00,
        },
    ),
    (
        "sonnet",
        Rate {
            provider: "anthropic",
            model: "claude-sonnet-5",
            usd_in: 2.00,
            usd_out: 10.00,
        },
    ),
    (
        "gpt",
        Rate {
            provider: "openai",
            model: "gpt-5.6-terra",
            usd_in: 2.00,
            usd_out: 12.00,
        },
    ),
];

/// The rate for a client nickname. `None` for anything unpriced — which is refused rather
/// than treated as free, because a model with no price is one whose spend nothing counts.
pub fn rate(model: &str) -> Option<&'static Rate> {
    let name = model.trim().to_ascii_lowercase();
    PRICES.iter().find(|(k, _)| *k == name).map(|(_, r)| r)
}

/// USD for a call of this shape. `None` for an unpriced model, which is refused rather
/// than quoted at nothing.
pub fn usd(tokens_in: usize, tokens_out: usize, model: &str) -> Option<f64> {
    Some(priced(tokens_in, tokens_out, rate(model)?))
}

/// The same arithmetic against a rate already in hand, so a caller that has checked the
/// model has no second chance to get `None` back and turn it into a free question.
fn priced(tokens_in: usize, tokens_out: usize, rate: &Rate) -> f64 {
    (tokens_in as f64 * rate.usd_in + tokens_out as f64 * rate.usd_out) / PER
}

/// What a question would cost, and what it would be answered from.
#[derive(Debug, Serialize)]
pub struct Quote {
    pub question: String,
    pub model: String,
    /// The calls whose transcripts would go in, shortest first.
    pub call_ids: Vec<String>,
    /// How many calls the sample held before the token cap trimmed it — the shortest
    /// `MAX_CALLS` of the selection that have a transcript, or fewer if the selection has
    /// fewer. `call_ids.len()` short of this is the one thing the browser cannot work out
    /// for itself: it means the context filled up, and the answer rests on fewer calls
    /// than the sample could have held.
    pub readable: usize,
    /// Input tokens, at the over-counting rate. The output side is `MAX_ANSWER_TOKENS` and
    /// is not shown: it is a ceiling the answer cannot pass, not a prediction.
    pub tokens_in: usize,
    pub usd: f64,
    /// The statistics, serialised. Not sent to the browser — it has them already, from
    /// `/api/stats` over the same filters — but this is the exact string the brain will be
    /// shown, and the string whose length was priced above.
    #[serde(skip)]
    pub stats: String,
}

/// Why a quote could not be given. The split is the one the API needs: a `Refused` is
/// something the caller can fix and reads as a 400, and anything else is the engine's own
/// failure and reads as a 500.
#[derive(Debug)]
pub enum Error {
    Refused(String),
    Failed(anyhow::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Refused(why) => write!(f, "{why}"),
            Error::Failed(e) => write!(f, "{e:#}"),
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Error::Failed(e)
    }
}

/// Price one question over one selection, and pick the calls it would read.
///
/// Nothing is spawned and nothing is written. Called twice per question that gets asked —
/// once for the price on the button, once when the button is pressed — so that the context
/// the answer is built from is the context that was priced, and not a set of ids that
/// travelled to a browser and back.
pub fn quote(db: &Db, f: &Filters, question: &str, model: &str) -> Result<Quote, Error> {
    let question = question.trim();
    if question.is_empty() {
        return Err(Error::Refused("a question cannot be empty".into()));
    }
    if question.chars().count() > MAX_QUESTION_CHARS {
        return Err(Error::Refused(format!(
            "a question is at most {MAX_QUESTION_CHARS} characters"
        )));
    }
    let model = model.trim().to_ascii_lowercase();
    let Some(rate) = rate(&model) else {
        let known: Vec<&str> = PRICES.iter().map(|(k, _)| *k).collect();
        return Err(Error::Refused(format!(
            "model must be one of {}, not {model}",
            known.join(", ")
        )));
    };

    let stats = serde_json::to_string(&queries::stats(db, f).map_err(Error::Failed)?)
        .map_err(|e| Error::Failed(e.into()))?;

    let budget = MAX_CONTEXT_TOKENS * CHARS_PER_TOKEN;
    // `len()` is bytes here and characters in the brain, so this over-counts wherever the
    // text is not ASCII — the direction a ceiling has to err.
    let mut chars = FIXED_PROMPT_CHARS + question.len() + stats.len();
    if chars >= budget {
        return Err(Error::Refused(format!(
            "this selection's statistics alone fill a question's context ({} tokens of \
             {MAX_CONTEXT_TOKENS}); narrow the window and ask again",
            chars / CHARS_PER_TOKEN
        )));
    }

    let samples = queries::transcripts(db, f, MAX_CALLS).map_err(Error::Failed)?;
    let readable = samples.len();
    let mut call_ids = Vec::new();
    for sample in samples {
        // Shortest first, so this stops at the first call that does not fit rather than
        // skipping it for a shorter one further down — there are none further down.
        let needs = sample.chars + FACTS_CHARS;
        if chars + needs > budget {
            break;
        }
        chars += needs;
        call_ids.push(sample.id);
    }

    let tokens_in = chars / CHARS_PER_TOKEN;
    Ok(Quote {
        question: question.to_string(),
        usd: priced(tokens_in, MAX_ANSWER_TOKENS, rate),
        model,
        call_ids,
        readable,
        tokens_in,
        stats,
    })
}
