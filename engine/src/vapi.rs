//! Vapi HTTP client. Read-only by construction: this file may only ever use
//! `reqwest::Client::get`, which `engine/tests/vapi.rs` enforces by grepping the source.
//! Source: https://docs.vapi.ai/api-reference/calls/list

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::time::Duration;

pub const DEFAULT_BASE: &str = "https://api.vapi.ai";

/// Vapi caps `limit` at 100 per page.
const PAGE: usize = 100;

/// Which calls to fetch. All bounds are optional except `last`, which is the hard stop.
#[derive(Debug, Default)]
pub struct FetchOpts {
    /// Stop once this many calls have been collected.
    pub last: usize,
    /// Stop at the first call whose `createdAt` is at or before this ISO-8601 instant.
    pub since: Option<String>,
    /// Start from calls created strictly before this ISO-8601 instant.
    pub until: Option<String>,
    /// Only calls belonging to this assistant.
    pub assistant_id: Option<String>,
}

/// How hard to retry a page that came back 429 or 5xx.
#[derive(Debug, Clone, Copy)]
pub struct Retry {
    /// Attempts after the first one.
    pub max: u32,
    /// Delay before the first retry; doubles each time.
    pub base_ms: u64,
}

impl Default for Retry {
    fn default() -> Self {
        Retry {
            max: 5,
            base_ms: 500,
        }
    }
}

/// Fetch calls from Vapi, newest first, in the order the API returns them.
pub async fn fetch_calls(key: &str, opts: &FetchOpts) -> Result<Vec<Value>> {
    fetch_calls_at(DEFAULT_BASE, key, opts, Retry::default()).await
}

/// `fetch_calls` against an explicit base URL and retry policy, so tests can point at a
/// mock server and not sleep for real seconds.
pub async fn fetch_calls_at(
    base: &str,
    key: &str,
    opts: &FetchOpts,
    retry: Retry,
) -> Result<Vec<Value>> {
    let http = client()?;
    let url = endpoint(base, "call");

    let mut extra: Vec<(&str, String)> = Vec::new();
    if let Some(id) = &opts.assistant_id {
        extra.push(("assistantId", id.clone()));
    }

    let mut out: Vec<Value> = Vec::new();
    let mut cursor = opts.until.clone();

    while out.len() < opts.last {
        let want = PAGE.min(opts.last - out.len());
        let page = get_page(&http, &url, "call", key, &extra, &cursor, want, retry).await?;

        // The cursor is the oldest `createdAt` on this page, and the `since` check below
        // can drop rows, so read it before anything is discarded.
        let oldest = page.iter().filter_map(created_at).min().map(str::to_string);

        let short = page.len() < want;
        let mut hit_since = false;
        for call in page {
            if let (Some(since), Some(at)) = (opts.since.as_deref(), created_at(&call)) {
                if at <= since {
                    hit_since = true;
                    break;
                }
            }
            out.push(call);
        }

        if hit_since || short || oldest.is_none() {
            break;
        }
        cursor = oldest;
    }

    out.truncate(opts.last);
    Ok(out)
}

/// Every row of a list endpoint that pages the way `/call` does — newest first, `limit`
/// up to 100, `createdAtLt` as the cursor. Used for `/tool`, `/assistant` and `/squad`,
/// which are small enough that there is never a reason to want part of them.
pub async fn fetch_all_at(
    base: &str,
    key: &str,
    resource: &str,
    retry: Retry,
) -> Result<Vec<Value>> {
    let http = client()?;
    let url = endpoint(base, resource);

    let mut out: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let page = get_page(&http, &url, resource, key, &[], &cursor, PAGE, retry).await?;
        let short = page.len() < PAGE;
        let oldest = page.iter().filter_map(created_at).min().map(str::to_string);
        out.extend(page);

        // `createdAtLt` is strictly-less-than, so a cursor that did not move means the
        // rows carry no usable `createdAt` and the next request would repeat this one.
        // There is no `last` here to stop the walk, so the walk has to stop itself.
        if short || oldest.is_none() || oldest == cursor {
            break;
        }
        cursor = oldest;
    }

    Ok(out)
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("building the HTTP client")
}

fn endpoint(base: &str, resource: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), resource)
}

fn created_at(row: &Value) -> Option<&str> {
    row.get("createdAt")?.as_str()
}

/// One page, retried on 429/5xx with doubling backoff.
#[allow(clippy::too_many_arguments)]
async fn get_page(
    http: &reqwest::Client,
    url: &str,
    resource: &str,
    key: &str,
    extra: &[(&str, String)],
    cursor: &Option<String>,
    want: usize,
    retry: Retry,
) -> Result<Vec<Value>> {
    let mut query: Vec<(&str, String)> = vec![("limit", want.to_string())];
    if let Some(at) = cursor {
        query.push(("createdAtLt", at.clone()));
    }
    query.extend(extra.iter().cloned());

    for attempt in 0..=retry.max {
        let res = http
            .get(url)
            .bearer_auth(key)
            .query(&query)
            .send()
            .await
            .with_context(|| format!("GET /{resource} failed"))?;
        let status = res.status();

        if status.is_success() {
            return res
                .json::<Vec<Value>>()
                .await
                .with_context(|| format!("GET /{resource} body"));
        }
        // Anything else is the caller's problem: a bad key, a bad filter. Do not retry.
        if status.as_u16() != 429 && !status.is_server_error() {
            bail!("GET /{resource} returned {status}");
        }
        if attempt < retry.max {
            tokio::time::sleep(Duration::from_millis(retry.base_ms << attempt)).await;
        }
    }

    bail!("GET /{resource} kept failing after {} retries", retry.max)
}
