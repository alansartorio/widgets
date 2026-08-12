//! Where the claude-usage daemon keeps its state.
//!
//! The layout and the env-var rules live in [`widgets_paths`]; all this adds is
//! the app name, the two system directories and the filenames inside them.
//!
//! The system flavour rather than the XDG one, and that is load-bearing: this
//! daemon runs as its own `claude-usage` user, with `/var/lib/claude-usage` and
//! `/run/claude-usage` from `StateDirectory=`/`RuntimeDirectory=` on a *system*
//! unit. No paths in the user's home are read, which is what lets the token live
//! somewhere the desktop session cannot reach.

use std::path::PathBuf;

pub use widgets_paths::{Paths, SOCKET_NAME};

/// The app name: the `CLAUDE_USAGE_*` env prefix.
pub const APP: &str = "claude-usage";
/// Default data directory, overridable with `CLAUDE_USAGE_DATA_DIR`.
pub const DEFAULT_DATA_DIR: &str = "/var/lib/claude-usage";
/// Default runtime directory, overridable with `CLAUDE_USAGE_RUNTIME_DIR`.
pub const DEFAULT_RUNTIME_DIR: &str = "/run/claude-usage";

/// The claude-usage-shaped constructor and file names, hung off the shared
/// `Paths` — an extension trait rather than a newtype, so `socket()` and the
/// directory fields stay reachable without forwarding each one by hand.
pub trait ClaudeUsagePaths {
    fn from_env() -> Paths;
    /// The SQLite database file.
    fn db(&self) -> PathBuf;
    /// The persisted credentials (a bare token or a `.credentials.json` body).
    fn credentials(&self) -> PathBuf;
    /// The refresh-throttle stamp file.
    fn refresh_stamp(&self) -> PathBuf;
}

impl ClaudeUsagePaths for Paths {
    fn from_env() -> Paths {
        Paths::system(APP, DEFAULT_DATA_DIR, DEFAULT_RUNTIME_DIR)
    }

    fn db(&self) -> PathBuf {
        self.data("claude-usage.db")
    }

    fn credentials(&self) -> PathBuf {
        self.data("credentials.json")
    }

    fn refresh_stamp(&self) -> PathBuf {
        self.data("refresh.stamp")
    }
}
