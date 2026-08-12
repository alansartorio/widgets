//! Shared library for the timers app: the timer model and its derived view,
//! the SQLite store, the timespec parser, and the bincode socket protocol used
//! by the daemon's clients (the CLI and the GUI).
//!
//! The daemon is the only process that touches the database or owns a deadline;
//! the CLI and GUI talk to it over a unix socket using the [`proto`] messages
//! and render whatever [`models::Status`] it hands back. Every value a client
//! displays — the countdown text, the percentage, the tooltip — is computed
//! once, in the daemon, by [`models::Status::from_timers`]. Clients do no time
//! arithmetic of their own beyond ticking a cached snapshot forward.

pub mod client;
pub mod db;
pub mod models;
pub mod paths;
pub mod proto;
pub mod timespec;

/// The current wall-clock time in unix seconds.
///
/// Deadlines are wall clock rather than monotonic on purpose: a timer set for
/// 15:30 has to still be a timer for 15:30 after a suspend, and one whose
/// deadline passed while the machine was asleep has to fire on resume rather
/// than have its remaining time silently extended.
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
