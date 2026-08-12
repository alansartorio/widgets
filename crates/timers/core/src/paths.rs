//! Where the timers daemon keeps its state.
//!
//! The layout and the env-var rules live in [`widgets_paths`]; all this adds is
//! the app name and the one filename inside it. The daemon is a *user* service,
//! so `StateDirectory=timers` and `RuntimeDirectory=timers` on its user unit
//! create exactly the XDG directories [`Paths::user`] resolves to — which is
//! why that unit sets no `TIMERS_*_DIR` at all. Those exist for running the
//! daemon by hand against a scratch directory.

use std::path::PathBuf;

pub use widgets_paths::{Paths, SOCKET_NAME};

/// The app name: the directory leaf and the `TIMERS_*` env prefix both.
pub const APP: &str = "timers";
/// The database name inside the data dir.
pub const DB_NAME: &str = "timers.db";

/// The timers-shaped constructor and file names, hung off the shared `Paths`.
///
/// An extension trait rather than a newtype so that everything `widgets_paths`
/// already answers — `socket()`, `data_dir`, `config()` — stays reachable
/// without a wrapper forwarding each one by hand.
pub trait TimersPaths {
    fn from_env() -> Paths;
    fn db(&self) -> PathBuf;
}

impl TimersPaths for Paths {
    fn from_env() -> Paths {
        Paths::user(APP)
    }

    fn db(&self) -> PathBuf {
        self.data(DB_NAME)
    }
}
