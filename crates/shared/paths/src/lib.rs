//! Where each tool keeps its state, its socket and its config.
//!
//! Two flavours, because the tools in this repo are split two ways. Most are
//! *user* services and live under the caller's XDG directories, which is
//! exactly what `StateDirectory=`/`RuntimeDirectory=` on a user unit create —
//! so those units set no environment at all and the defaults land in the right
//! place on their own. claude-usage is the exception: it runs as its own system
//! user with `/var/lib/claude-usage` and `/run/claude-usage`, and reads nothing
//! out of anyone's home.
//!
//! Note the deliberate asymmetry between the two sources of a directory: an env
//! override *is* the directory, while the XDG fallback appends the app name. So
//! `TIMERS_DATA_DIR=/tmp/t` uses `/tmp/t`, not `/tmp/t/timers` — which is what
//! makes running a daemon by hand against a scratch directory predictable, and
//! what every test in this workspace relies on.

use std::path::PathBuf;

/// The socket name inside the runtime dir. The same for every tool: they are
/// told apart by the directory, not the file.
pub const SOCKET_NAME: &str = "control.sock";

/// Where everything one tool owns lives. Constructed from the environment so
/// the same binary runs the same way under systemd and by hand for testing,
/// with the env vars pointed somewhere writable.
#[derive(Clone, Debug)]
pub struct Paths {
    /// State that survives reboots: databases, caches that cannot be refetched
    /// offline, credentials.
    pub data_dir: PathBuf,
    /// Ephemeral state: the control socket, last-value files.
    pub runtime_dir: PathBuf,
    /// What the user (or home-manager) writes: config files, override files.
    pub config_dir: PathBuf,
}

impl Paths {
    /// The user-service flavour: XDG directories with a per-app leaf.
    ///
    /// `app` is both the leaf and the env-var prefix — `"claude-usage"` reads
    /// `CLAUDE_USAGE_DATA_DIR`, `"timers"` reads `TIMERS_DATA_DIR`.
    pub fn user(app: &str) -> Self {
        let prefix = env_prefix(app);
        Self {
            data_dir: dir(&format!("{prefix}_DATA_DIR"), "XDG_STATE_HOME", app, || {
                home().join(".local/state")
            }),
            // There is no sensible non-XDG fallback for a runtime dir: a
            // world-writable /tmp path would let anyone on the box preempt the
            // socket. $XDG_RUNTIME_DIR is set in every session that could run
            // this, so falling back into the state dir is the safe answer for
            // the odd case where it isn't — the socket ends up somewhere
            // private, even if it outlives a reboot.
            runtime_dir: dir(
                &format!("{prefix}_RUNTIME_DIR"),
                "XDG_RUNTIME_DIR",
                app,
                || home().join(".local/state"),
            ),
            config_dir: dir(
                &format!("{prefix}_CONFIG_DIR"),
                "XDG_CONFIG_HOME",
                app,
                || home().join(".config"),
            ),
        }
    }

    /// The system-service flavour: fixed directories, same env overrides.
    ///
    /// Used by claude-usage, whose daemon runs as its own user and must not
    /// read the user's home — the directories are what `StateDirectory=` and
    /// `RuntimeDirectory=` create for a *system* unit.
    pub fn system(app: &str, data: &str, runtime: &str) -> Self {
        let prefix = env_prefix(app);
        Self {
            data_dir: fixed(&format!("{prefix}_DATA_DIR"), data),
            runtime_dir: fixed(&format!("{prefix}_RUNTIME_DIR"), runtime),
            // /etc/xdg is where a NixOS module drops a config for a service
            // that has no home directory to put one in.
            config_dir: fixed(&format!("{prefix}_CONFIG_DIR"), &format!("/etc/xdg/{app}")),
        }
    }

    /// The control socket.
    pub fn socket(&self) -> PathBuf {
        self.runtime_dir.join(SOCKET_NAME)
    }

    /// A file in the data dir.
    pub fn data(&self, name: &str) -> PathBuf {
        self.data_dir.join(name)
    }

    /// A file in the runtime dir.
    pub fn runtime(&self, name: &str) -> PathBuf {
        self.runtime_dir.join(name)
    }

    /// A file in the config dir.
    pub fn config(&self, name: &str) -> PathBuf {
        self.config_dir.join(name)
    }
}

/// `claude-usage` → `CLAUDE_USAGE`. Hyphens are legal in a package name and not
/// in an environment variable, so they become underscores.
fn env_prefix(app: &str) -> String {
    app.to_uppercase().replace('-', "_")
}

/// `$override` verbatim if set, else `$base/<app>`, else `fallback()/<app>`.
fn dir(
    override_var: &str,
    base_var: &str,
    app: &str,
    fallback: impl FnOnce() -> PathBuf,
) -> PathBuf {
    if let Some(dir) = non_empty(override_var) {
        return PathBuf::from(dir);
    }
    let base = non_empty(base_var)
        .map(PathBuf::from)
        .unwrap_or_else(fallback);
    base.join(app)
}

/// `$override` verbatim if set, else the literal default.
fn fixed(override_var: &str, default: &str) -> PathBuf {
    non_empty(override_var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

/// An env var's value, treating unset and empty the same — systemd and shells
/// both export empty strings for "not configured" often enough that an empty
/// `XDG_STATE_HOME` must not resolve to `/timers`.
fn non_empty(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.is_empty())
}

fn home() -> PathBuf {
    non_empty("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_prefix_uppercases_and_replaces_hyphens() {
        assert_eq!(env_prefix("timers"), "TIMERS");
        assert_eq!(env_prefix("claude-usage"), "CLAUDE_USAGE");
    }

    #[test]
    fn an_override_is_the_directory_not_its_parent() {
        // The whole point of the asymmetry: a scratch dir passed in by a test
        // or a hand-run daemon is used verbatim, with no leaf appended.
        let got = dir(
            "WIDGETS_TEST_UNSET_XX",
            "WIDGETS_TEST_BASE_XX",
            "timers",
            || PathBuf::from("/fallback"),
        );
        assert_eq!(got, PathBuf::from("/fallback/timers"));
    }
}
