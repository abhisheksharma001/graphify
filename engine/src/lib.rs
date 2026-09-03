//! Engine internals. Lives in a library target so `engine/tests/` can reach it;
//! `main.rs` is the thin CLI shell on top.

pub mod db;
pub mod ended_reason;
pub mod extract;
pub mod vapi;
