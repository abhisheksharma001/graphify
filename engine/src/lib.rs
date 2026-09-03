//! Engine internals. Lives in a library target so `engine/tests/` can reach it;
//! `main.rs` is the thin CLI shell on top.

pub mod assistants;
pub mod db;
pub mod ended_reason;
pub mod extract;
pub mod sync;
pub mod vapi;

/// The clock, in one place, so the modules that stamp rows do not each own one and the
/// pure ones (`extract`) can go on owning none. Same shape as Vapi's own timestamps, so a
/// `synced_at` or a `fetched_at` compares directly against a `created_at`.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
