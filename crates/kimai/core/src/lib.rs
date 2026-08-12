//! The kimai integration, minus the two front ends.
//!
//! `kimai-api` knows how to talk to Kimai and `holidays` knows what a working
//! day is; this is where the two meet and become the numbers the bar shows. The
//! CLI and the GUI are both thin over it — deliberately, since they are two
//! processes computing the same thing from the same endpoints, and any
//! arithmetic that lived in one of them would eventually disagree with the
//! other.
//!
//! No daemon, unlike timers and claude-usage. There is nothing to keep between
//! polls that Kimai is not already the authority on, no notification to fire
//! and no state to own — a daemon here would be a cache with a socket in front
//! of it. So `kimai tail` polls for the bar, `kimai-gui` polls for its window
//! while it is open, and the only thing they share is a file the former drops
//! for the latter to paint from before its own first request returns.

pub mod config;
pub mod emitter;
pub mod output;
pub mod poller;

pub use config::Config;
pub use emitter::{Emitter, describe};
pub use output::{Output, State, format_duration, format_signed};
pub use poller::Poller;

use std::path::PathBuf;

/// Where `kimai tail` mirrors its last line for the GUI to open on.
pub fn last_line_path() -> PathBuf {
    widgets_paths::Paths::user("kimai").runtime("last.json")
}
