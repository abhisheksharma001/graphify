//! The rule DSL: what a pattern is, and what running one over a call means.
//!
//! A rule is data — a JSON object with a fixed set of keys — and this module is the only
//! thing in the engine that reads one. Nothing here evaluates anything: the closest it
//! comes is a regular expression, and `regex` has no backreferences and no backtracking,
//! so a rule a model wrote cannot be made to run for ever.
//!
//! Two halves. Everything above `apply` is pure and has never heard of SQLite, which is
//! what lets a test state a rule, a transcript and the answer with no database in the way.
//! `apply` is the driver under the `graphify apply` subcommand, and owns the only SQL.

use crate::db::Db;
use anyhow::{bail, Context, Result};
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

/// Ceiling on one compiled regex. A model writes these, and a pathological pattern can
/// compile to an enormous automaton without ever looking wrong. 1 MB is far past anything
/// a phrase-matching rule needs.
const REGEX_BYTES: usize = 1 << 20;

/// The DSL, exactly as `docs/spec.md` spells it and exactly as `SynthesizeRule` returns
/// it. Every field is optional; an empty rule matches every call.
///
/// **A list means "any of these".** `ended_reasons` holds alternatives, not requirements,
/// and so do `tool_called` and the rest — which makes `tool_not_called` the plain negation
/// of `tool_called` rather than a second thing to learn.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Rule {
    /// Case-insensitive substrings of one line of the transcript.
    pub any_phrases: Vec<String>,
    /// Regular expressions over the same lines. Compiled case-insensitively — a
    /// transcript is speech recognition output, and a rule written in lower case must not
    /// miss a capitalised sentence. `(?-i)` turns that back off for one pattern.
    pub regex: Vec<String>,
    /// `user`, `bot`, or `any`. Absent is `any`. Filters which lines the phrases and the
    /// regexes are allowed to look at, and nothing else.
    pub speaker: Option<String>,
    pub ended_reasons: Vec<String>,
    pub ended_groups: Vec<String>,
    pub tool_called: Vec<String>,
    pub tool_not_called: Vec<String>,
    /// `true`: at least one tool call failed. `false`: none did.
    pub tool_failed: Option<bool>,
    pub transferred: Option<bool>,
    pub min_duration_s: Option<f64>,
    pub max_duration_s: Option<f64>,
}

/// Everything a rule is allowed to look at, and the shape `rule-check --calls` reads.
///
/// Narrower than a `calls` row on purpose: a rule has no business seeing costs, latencies
/// or the slim payload, and `apply` should not drag them off disk to find that out.
///
/// Unknown keys are refused for the same reason they are in a rule: this file is a
/// contract with the brain, and a `transcripts` where `transcript` was meant would answer
/// every question about words with "no" and look like a rule that simply did not match.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Subject {
    pub id: String,
    pub transcript: Option<String>,
    pub ended_reason: Option<String>,
    pub ended_group: Option<String>,
    pub transferred: Option<bool>,
    pub duration_s: Option<f64>,
    pub tool_calls: Vec<Tool>,
}

/// The part of a `tool_calls` row a rule can see: which tool, and whether it failed.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Tool {
    pub name: Option<String>,
    pub failed: Option<bool>,
}

/// Which side of the conversation a line came from. `Other` is the system line and
/// anything else Vapi labels: a real speaker, so it starts a turn, but neither of the two
/// the DSL can ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Side {
    User,
    Bot,
    Other,
}

/// A rule that has been checked over: its regexes are compiled and its speaker is a word
/// this module knows.
///
/// [`matches`] takes one of these rather than a bare [`Rule`] so a rule that cannot work
/// fails once, loudly, where it was loaded — never as a quiet `false` on every call in the
/// org — and so `apply` compiles each regex once instead of once per call.
#[derive(Debug)]
pub struct Checked {
    rule: Rule,
    regexes: Vec<Regex>,
    /// `None` is `any`: no filter at all, rather than a third side.
    speaker: Option<Side>,
}

/// Parse a rule and compile it, or say what is wrong with it and name the pattern it
/// belongs to.
///
/// Unknown keys are an error rather than something to ignore. A model that invents
/// `min_turns` has to be told: a silently dropped condition leaves a rule that matches
/// calls it was never meant to, and the count it feeds looks perfectly reasonable.
///
/// `name` opens every message here and is used verbatim, so `apply` can say which pattern
/// and `rule-check` can name the file. The caller that hits this is `apply`, running
/// patterns it did not write, and "bad regex" with nothing attached is a search.
pub fn validate(json: &str, name: &str) -> Result<Checked> {
    let rule: Rule =
        serde_json::from_str(json).with_context(|| format!("{name}: not a valid rule"))?;

    let speaker = match rule.speaker.as_deref().map(str::trim).map(str::to_lowercase) {
        None => None,
        Some(word) => match word.as_str() {
            "any" | "" => None,
            "user" => Some(Side::User),
            "bot" => Some(Side::Bot),
            other => bail!("{name}: speaker must be user, bot or any, not {other:?}"),
        },
    };

    // An empty phrase is a substring of every line, so a rule holding one matches the
    // whole org while looking like it selects something.
    if let Some(blank) = rule.any_phrases.iter().find(|p| p.trim().is_empty()) {
        bail!("{name}: an empty phrase would match every call: {blank:?}");
    }

    let mut regexes = Vec::with_capacity(rule.regex.len());
    for pattern in &rule.regex {
        regexes.push(
            RegexBuilder::new(pattern)
                .case_insensitive(true)
                .size_limit(REGEX_BYTES)
                .build()
                .with_context(|| format!("{name}: bad regex {pattern:?}"))?,
        );
    }

    Ok(Checked {
        rule,
        regexes,
        speaker,
    })
}

/// Does this call match this rule?
///
/// The text half and the structural half, both of which must hold. Spelled out in the
/// spec as: any phrase or any regex on lines from `speaker` (both empty = true), AND every
/// non-null, non-empty structural condition.
pub fn matches(rule: &Checked, call: &Subject) -> bool {
    text_hit(rule, call) && structure_holds(rule, call)
}

fn text_hit(rule: &Checked, call: &Subject) -> bool {
    if rule.rule.any_phrases.is_empty() && rule.regexes.is_empty() {
        return true;
    }
    // A rule that asks about words cannot be answered about a call with no transcript.
    // Unknown is not "no match" for a good reason, but a count has to put it somewhere,
    // and counting it as a match would invent the evidence.
    let Some(transcript) = call.transcript.as_deref() else {
        return false;
    };
    let phrases: Vec<String> = rule
        .rule
        .any_phrases
        .iter()
        .map(|p| p.trim().to_lowercase())
        .collect();

    turns(transcript)
        .iter()
        .filter(|(side, _)| rule.speaker.is_none_or(|want| *side == want))
        .any(|(_, text)| {
            let lower = text.to_lowercase();
            phrases.iter().any(|p| lower.contains(p))
                || rule.regexes.iter().any(|r| r.is_match(text))
        })
}

fn structure_holds(checked: &Checked, call: &Subject) -> bool {
    let rule = &checked.rule;

    if !rule.ended_reasons.is_empty() && !listed(&rule.ended_reasons, call.ended_reason.as_deref())
    {
        return false;
    }
    if !rule.ended_groups.is_empty() && !listed(&rule.ended_groups, call.ended_group.as_deref()) {
        return false;
    }
    if !rule.tool_called.is_empty() && !called(call, &rule.tool_called) {
        return false;
    }
    if !rule.tool_not_called.is_empty() && called(call, &rule.tool_not_called) {
        return false;
    }
    if let Some(want) = rule.tool_failed {
        let failed = call.tool_calls.iter().any(|t| t.failed == Some(true));
        if failed != want {
            return false;
        }
    }
    // `Some(want)`, not `unwrap_or(false)`: a call whose `transferred` is NULL is a call
    // nobody knows about, and it must not answer either question. Same rule as the UI's
    // NULL → "—".
    if let Some(want) = rule.transferred {
        if call.transferred != Some(want) {
            return false;
        }
    }
    if let Some(min) = rule.min_duration_s {
        if !call.duration_s.is_some_and(|d| d >= min) {
            return false;
        }
    }
    if let Some(max) = rule.max_duration_s {
        if !call.duration_s.is_some_and(|d| d <= max) {
            return false;
        }
    }
    true
}

/// Is `value` one of `list`? Compared without case and without surrounding space: these
/// are identifiers a model copied out of a prompt, and a capital letter is not a different
/// tool. A missing value is in no list — NULL is not a member of anything.
fn listed(list: &[String], value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let value = value.trim();
    list.iter().any(|want| want.trim().eq_ignore_ascii_case(value))
}

fn called(call: &Subject, names: &[String]) -> bool {
    call.tool_calls
        .iter()
        .any(|t| listed(names, t.name.as_deref()))
}

/// Who Vapi puts in front of a line. Mirrors the parser in `ui/src/CallDrawer.tsx`,
/// deliberately word for word: the drawer a reader checks a match against and the rule
/// that produced it must agree on who said what.
fn side(head: &str) -> Option<Side> {
    match head.trim().to_ascii_lowercase().as_str() {
        "user" | "human" => Some(Side::User),
        "ai" | "bot" | "assistant" => Some(Side::Bot),
        "system" => Some(Side::Other),
        _ => None,
    }
}

/// The transcript as turns. Vapi sends one string of `Speaker: line` rows; a line whose
/// head is a known speaker starts a turn and **anything else continues the previous one**,
/// so a colon inside a sentence never invents a speaker and no line is ever dropped.
///
/// A transcript that opens with an unlabelled line has a turn nobody claimed. It is
/// `Other`, so a rule asking about the user or the bot passes over it rather than
/// guessing which of them it was.
fn turns(transcript: &str) -> Vec<(Side, String)> {
    let mut out: Vec<(Side, String)> = Vec::new();
    for line in transcript.lines() {
        if let Some(at) = line.find(':') {
            if let Some(side) = side(&line[..at]) {
                out.push((side, line[at + 1..].trim().to_string()));
                continue;
            }
        }
        let rest = line.trim();
        if rest.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some((_, text)) => {
                text.push('\n');
                text.push_str(rest);
            }
            None => out.push((Side::Other, rest.to_string())),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The driver. Everything above this line is pure.
// ---------------------------------------------------------------------------

/// What one pattern did, for the line `graphify apply` prints about it.
#[derive(Debug)]
pub struct Applied {
    pub name: String,
    pub matched: usize,
    pub of: usize,
}

/// A `patterns` row as `apply` needs it.
struct Stored {
    id: i64,
    org_id: Option<i64>,
    name: String,
    rule: Option<String>,
}

/// Re-run every free-mode pattern's rule over its org's calls, replacing that pattern's
/// `source='rule'` matches.
///
/// Only `mode='free'`. Those are the patterns a rule alone decides, so re-counting them is
/// arithmetic and costs nothing — which is the whole reason to have a rule engine. Hybrid
/// and full patterns have a model in the loop and a daily cap on it; re-running one is
/// spending money, and no plain `graphify apply` is going to do that.
///
/// A pattern with no rule yet, or with no org, is skipped rather than an error: it is a
/// row someone is halfway through creating, not a mistake. A pattern whose rule is broken
/// **is** an error, and it names the pattern.
pub fn apply(db: &mut Db) -> Result<Vec<Applied>> {
    let patterns = free_patterns(db)?;
    // One org's calls are read once however many of its patterns want them. The transcript
    // column is the biggest thing here, and reading it per pattern is the easy waste.
    let mut by_org: HashMap<i64, Vec<Subject>> = HashMap::new();
    let mut report = Vec::new();

    for pattern in patterns {
        let (Some(org_id), Some(json)) = (pattern.org_id, pattern.rule.as_deref()) else {
            continue;
        };
        if let Entry::Vacant(slot) = by_org.entry(org_id) {
            slot.insert(subjects(db, org_id)?);
        }
        report.push(run_one(db, pattern.id, &pattern.name, json, &by_org[&org_id])?);
    }
    Ok(report)
}

/// Re-run one pattern's rule, whatever mode it is in.
///
/// `apply` skips the modes with a model in the loop because re-counting one of those means
/// spending; this does not, because it is only ever the rule half. It replaces the same
/// `source='rule'` rows and leaves the model's answers where they are, so a hybrid pattern
/// asked to re-apply gets its rule re-run and nothing bought.
///
/// `None` for a pattern with no rule yet or no org: a row someone is halfway through
/// making is not an error.
pub fn apply_one(db: &mut Db, id: i64) -> Result<Option<Applied>> {
    let Some(pattern) = db.pattern(id)? else {
        return Ok(None);
    };
    let (Some(org_id), Some(json)) = (pattern.org_id, pattern.rule.as_deref()) else {
        return Ok(None);
    };
    let name = pattern.name.unwrap_or_else(|| format!("#{id}"));
    let calls = subjects(db, org_id)?;
    Ok(Some(run_one(db, id, &name, json, &calls)?))
}

/// One pattern over one org's calls: validate, match, store, count.
fn run_one(
    db: &mut Db,
    id: i64,
    name: &str,
    json: &str,
    calls: &[Subject],
) -> Result<Applied> {
    let rule = validate(json, &format!("pattern {name}"))?;
    let hits: Vec<String> = calls
        .iter()
        .filter(|c| matches(&rule, c))
        .map(|c| c.id.clone())
        .collect();
    db.replace_rule_matches(id, &hits)?;
    Ok(Applied {
        name: name.to_string(),
        matched: hits.len(),
        of: calls.len(),
    })
}

fn free_patterns(db: &Db) -> Result<Vec<Stored>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT id, org_id, name, rule FROM patterns WHERE mode = 'free' ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        let id: i64 = r.get(0)?;
        Ok(Stored {
            id,
            org_id: r.get(1)?,
            // A pattern need not be named yet, and an error still has to point at it.
            name: r
                .get::<_, Option<String>>(2)?
                .unwrap_or_else(|| format!("#{id}")),
            rule: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Every call in the org, with the six columns a rule can read and its tool calls.
fn subjects(db: &Db, org_id: i64) -> Result<Vec<Subject>> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT id, transcript, ended_reason, ended_group, transferred, duration_s
           FROM calls WHERE org_id = ?1",
    )?;
    let mut calls: Vec<Subject> = stmt
        .query_map([org_id], |r| {
            Ok(Subject {
                id: r.get(0)?,
                transcript: r.get(1)?,
                ended_reason: r.get(2)?,
                ended_group: r.get(3)?,
                transferred: r.get(4)?,
                duration_s: r.get(5)?,
                tool_calls: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Every tool call in the org in one query, not one query per call: a thousand calls is
    // otherwise a thousand round trips to answer a question about a handful of them.
    let mut stmt = conn.prepare(
        "SELECT t.call_id, t.name, t.failed
           FROM tool_calls t JOIN calls c ON c.id = t.call_id
          WHERE c.org_id = ?1",
    )?;
    let rows = stmt.query_map([org_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            Tool {
                name: r.get(1)?,
                failed: r.get(2)?,
            },
        ))
    })?;
    let mut tools: HashMap<String, Vec<Tool>> = HashMap::new();
    for row in rows {
        let (call_id, tool) = row?;
        tools.entry(call_id).or_default().push(tool);
    }
    for call in &mut calls {
        if let Some(rows) = tools.remove(&call.id) {
            call.tool_calls = rows;
        }
    }
    Ok(calls)
}
