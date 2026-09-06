//! What the operator has to be told, kept where the failure cannot reach it.
//!
//! Two things in this engine can fail in a way nobody is watching: the boot sweep that
//! clears jobs a dead process left live (S-40), and the close that ends a job (S-39). Both
//! are the same failure underneath — SQLite refusing a write to `jobs` — and both have
//! until now been said to stderr, which is nowhere in a product that is a browser tab.
//!
//! That shared cause is also what decides where a notice can live. **A notice about a
//! failed write cannot be written to the database that failed the write.** There is no
//! table for this and there cannot be one, so the board is in memory and lasts exactly as
//! long as the process.
//!
//! Which is the right lifetime rather than a compromise. The sweep's own sentence tells the
//! operator to make the database writable and start graphify again, and a restart re-runs
//! the sweep: it either clears the rows, in which case the board is empty and truthfully
//! so, or it fails again and says so again. A board that cannot go stale needs no way to
//! dismiss it. The restart is the dismissal.

use std::collections::VecDeque;
use std::sync::Mutex;

/// One thing that went wrong, as the person reading it needs it: when, and what it cost.
/// No id, no severity and no source — this is a list of two possible sentences, not a
/// logging system, and every field nothing reads is a field that invites a third caller.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Notice {
    pub at: String,
    pub text: String,
}

/// The kept notices and what did not fit, under one lock so that nothing can read a list
/// and a count that were taken a push apart.
#[derive(Debug, Default)]
struct Board {
    kept: VecDeque<Notice>,
    dropped: usize,
}

/// The notices this process has to show, newest first.
#[derive(Debug, Default)]
pub struct Notices {
    board: Mutex<Board>,
}

impl Notices {
    /// Twenty of the same sentence says nothing the first one did not. The number bounds
    /// memory when a database is refusing every write and so failing every close; it is not
    /// a judgement about which notices matter. The oldest is dropped because it is the one
    /// whose cause is most likely already over.
    pub const KEEP: usize = 20;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, text: impl Into<String>) {
        let notice = Notice {
            at: crate::now(),
            text: text.into(),
        };
        let mut board = self.lock();
        board.kept.push_front(notice);
        while board.kept.len() > Self::KEEP {
            board.kept.pop_back();
            // Counted rather than truncated in silence. A bound that quietly loses things
            // is the swallowed count this register has spent three steps taking out.
            board.dropped += 1;
        }
    }

    /// The board, newest first, and how many notices it could not keep.
    pub fn all(&self) -> (Vec<Notice>, usize) {
        let board = self.lock();
        (board.kept.iter().cloned().collect(), board.dropped)
    }

    /// A poisoned lock means a caller panicked mid-push, which leaves a `VecDeque` and a
    /// `usize` and neither can be half-written. Recovering is the reasoning
    /// `server::App::db` already gives about the database itself.
    fn lock(&self) -> std::sync::MutexGuard<'_, Board> {
        self.board.lock().unwrap_or_else(|e| e.into_inner())
    }
}
