//! Everything that used to be a literal in this program's source.
//!
//! The binary this replaces knew one lamp — `light.desk_lamp`, spelled out
//! in `main`— wrote to two paths under `/tmp`, moved by exactly ten percent per
//! scroll notch and refused to go below 0.39% brightness, all as constants. A
//! second lamp, or a machine where `/tmp` is shared with anything, meant a
//! patch. So all of it is here, and every value below defaults to what that
//! source hardcoded except the two socket paths — see [`Config::command_socket`].
//!
//! The token is a *path*, like everywhere else in this workspace: sops decrypts
//! it into `/run/secrets` and this reads it there at start, so nothing secret
//! lands in `~/.config` or in the nix store. The launcher that used to
//! `cat` the token into `HASS_TOKEN` before exec'ing is what this replaces —
//! that put the plaintext in a process environment any process of the same user
//! could read.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const APP: &str = "hass";
pub const FILE_NAME: &str = "config.toml";

/// The socket the bar sends commands to, inside the runtime dir.
pub const COMMAND_SOCKET_NAME: &str = "command.sock";
/// The socket the bar reads widget state from, inside the runtime dir.
pub const STATE_SOCKET_NAME: &str = "state.sock";

/// Per-user, both of them: nothing else on the machine has any business
/// dimming this session's lamp or watching it be dimmed.
pub const SOCKET_MODE: u32 = 0o600;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// e.g. `http://homeassistant.local:8123`.
    pub base_url: String,

    /// A file holding a long-lived access token. Read at start, not baked in.
    ///
    /// Optional only because `$HASS_TOKEN` is an accepted substitute for running
    /// this by hand; one of the two has to be there. See [`Config::token`].
    #[serde(default)]
    pub token_file: Option<PathBuf>,

    /// The light to drive, e.g. `light.desk_lamp`.
    ///
    /// Required, and deliberately without a default: the previous default was
    /// one particular lamp in one particular bedroom, which is not a fact this
    /// repo should hold.
    pub entity: String,

    /// Where the bar sends `increase`, `toggle_state` and friends. A *datagram*
    /// socket — see [`crate::proto`].
    ///
    /// Defaults to `$XDG_RUNTIME_DIR/hass/command.sock`, which is a change from
    /// the `/tmp/hass-light.sock` the old launcher used: a world-writable
    /// directory lets anything on the box preempt the path and collect the
    /// commands. Anyone who would rather keep the old path can say so here, but
    /// the eww yuck has to name one of them either way.
    #[serde(default)]
    pub command_socket: Option<PathBuf>,

    /// Where the bar reads widget state from, as one JSON object per line. A
    /// stream socket, and every connected client gets every update.
    ///
    /// Defaults to `$XDG_RUNTIME_DIR/hass/state.sock`, replacing
    /// `/tmp/hass-light-eww-output.sock`.
    #[serde(default)]
    pub state_socket: Option<PathBuf>,

    /// How far one scroll notch moves brightness or colour temperature, in
    /// percentage points.
    #[serde(default = "default_step")]
    pub step_percent: f32,

    /// The floor on brightness, in percent.
    ///
    /// Home Assistant takes brightness as 1–255, so 0.39 (= 1/255) is the
    /// dimmest a lamp can be while still being on. Clamping here rather than at
    /// zero is what keeps scrolling all the way down from turning the lamp off
    /// behind the widget's back.
    #[serde(default = "default_min_brightness")]
    pub min_brightness_percent: f32,

    /// How long to wait between polls while the lamp is coming up.
    ///
    /// A lamp that was just turned on reports no brightness until it has
    /// actually reached one, so the widget shows `loading` and asks again on
    /// this interval until the answer arrives.
    #[serde(default = "default_settle_poll_ms")]
    pub settle_poll_ms: u64,

    /// How long to keep asking before giving up and leaving the pill on
    /// `loading` until the next command.
    ///
    /// The binary this replaces had no ceiling here: it polled every 100 ms
    /// until Home Assistant said the lamp was on, which against an unreachable
    /// bulb is ten requests a second, forever.
    #[serde(default = "default_settle_timeout_ms")]
    pub settle_timeout_ms: u64,

    /// How long any one HTTP request may take before it is given up on.
    #[serde(default = "default_request_timeout")]
    pub request_timeout_seconds: u64,
}

fn default_step() -> f32 {
    10.0
}
fn default_min_brightness() -> f32 {
    0.39
}
fn default_settle_poll_ms() -> u64 {
    100
}
fn default_settle_timeout_ms() -> u64 {
    3_000
}
fn default_request_timeout() -> u64 {
    crate::api::REQUEST_TIMEOUT.as_secs()
}

impl Config {
    /// Every place a config could be, in the order they win.
    ///
    /// The same arrangement as `kimai`: `$XDG_CONFIG_DIRS` is in there for the
    /// NixOS flavour of the module, which drops the file in `/etc/xdg/hass` on a
    /// machine without home-manager.
    pub fn search_paths() -> Vec<PathBuf> {
        if let Ok(explicit) = std::env::var("HASS_CONFIG")
            && !explicit.is_empty()
        {
            return vec![PathBuf::from(explicit)];
        }

        let mut out = vec![widgets_paths::Paths::user(APP).config(FILE_NAME)];
        if let Ok(dirs) = std::env::var("XDG_CONFIG_DIRS") {
            out.extend(
                dirs.split(':')
                    .filter(|d| !d.is_empty())
                    .map(|d| Path::new(d).join(APP).join(FILE_NAME)),
            );
        } else {
            out.push(PathBuf::from("/etc/xdg/hass").join(FILE_NAME));
        }
        out
    }

    /// Load the first config that exists.
    ///
    /// `Ok(None)` means there is none. Unlike `kimai`, whose CLI prints
    /// `configured: false` at the bar, the caller here is a daemon: no config is
    /// nothing to run, and it should say so and exit rather than sit there.
    pub fn load() -> Result<Option<Self>> {
        for path in Self::search_paths() {
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let cfg: Self = toml_edit::de::from_str(&raw)
                .with_context(|| format!("reading {}", path.display()))?;
            return Ok(Some(cfg.with_env()));
        }
        Ok(None)
    }

    /// Let the environment win, for running it by hand against another instance
    /// without editing the file home-manager owns.
    ///
    /// `HASS_HOST` is the name the old launcher exported, kept as an alias so a
    /// half-migrated machine does not silently talk to the wrong Home Assistant.
    pub fn with_env(mut self) -> Self {
        for var in ["HASS_URL", "HASS_HOST"] {
            if let Ok(url) = std::env::var(var)
                && !url.is_empty()
            {
                self.base_url = url;
            }
        }
        if let Ok(file) = std::env::var("HASS_TOKEN_FILE")
            && !file.is_empty()
        {
            self.token_file = Some(PathBuf::from(file));
        }
        if let Ok(entity) = std::env::var("HASS_ENTITY")
            && !entity.is_empty()
        {
            self.entity = entity;
        }
        self
    }

    /// The token, from `$HASS_TOKEN` if it is set and from `token_file`
    /// otherwise.
    ///
    /// The env var is for hand-running and loses to nothing, because a config
    /// file is what the module writes and a person who exported a token meant
    /// it. Read fresh at every start; a rotated token means restarting the
    /// daemon, which is the same bargain every other tool here makes.
    pub fn token(&self) -> Result<String> {
        if let Ok(token) = std::env::var("HASS_TOKEN")
            && !token.is_empty()
        {
            return Ok(token);
        }
        let Some(path) = &self.token_file else {
            bail!("no token: set token_file in the config or $HASS_TOKEN in the environment");
        };
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading the token at {}", path.display()))?;
        Ok(raw.trim().to_string())
    }

    pub fn client(&self) -> Result<crate::api::Client> {
        Ok(crate::api::Client::new(
            &self.base_url,
            self.token()?,
            self.request_timeout(),
        ))
    }

    pub fn lamp(&self) -> Result<crate::lamp::Lamp> {
        Ok(crate::lamp::Lamp::new(
            self.client()?,
            self.entity.clone(),
            self.min_brightness_percent,
        ))
    }

    fn paths(&self) -> widgets_paths::Paths {
        widgets_paths::Paths::user(APP)
    }

    pub fn command_socket(&self) -> PathBuf {
        self.command_socket
            .clone()
            .unwrap_or_else(|| self.paths().runtime(COMMAND_SOCKET_NAME))
    }

    pub fn state_socket(&self) -> PathBuf {
        self.state_socket
            .clone()
            .unwrap_or_else(|| self.paths().runtime(STATE_SOCKET_NAME))
    }

    /// The step, floored well above zero: a step of nothing is a widget that
    /// scrolls and does not move, which reads as a broken lamp rather than a
    /// misconfigured file.
    pub fn step(&self) -> f32 {
        self.step_percent.abs().max(0.1)
    }

    pub fn settle_poll_interval(&self) -> Duration {
        Duration::from_millis(self.settle_poll_ms.max(10))
    }

    /// Never shorter than one poll, so a timeout can never mean "do not look at
    /// all" — the one refetch after switching a lamp on is the whole point.
    pub fn settle_timeout(&self) -> Duration {
        Duration::from_millis(self.settle_timeout_ms).max(self.settle_poll_interval())
    }

    pub fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_seconds.max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Config {
        toml_edit::de::from_str(raw).unwrap()
    }

    const MINIMAL: &str = r#"
        base_url = "http://hass.example:8123"
        token_file = "/run/secrets/hass_token"
        entity = "light.desk_lamp"
    "#;

    #[test]
    fn the_defaults_are_what_the_old_source_hardcoded() {
        let cfg = parse(MINIMAL);
        assert_eq!(cfg.step(), 10.0);
        assert_eq!(cfg.min_brightness_percent, 0.39);
        assert_eq!(cfg.settle_poll_interval(), Duration::from_millis(100));
    }

    #[test]
    fn the_sockets_default_under_the_runtime_dir_not_tmp() {
        let cfg = parse(MINIMAL);
        // The leaf names are the contract with the eww yuck; the parent comes
        // from $XDG_RUNTIME_DIR, which a test cannot assume.
        assert!(cfg.command_socket().ends_with("hass/command.sock"));
        assert!(cfg.state_socket().ends_with("hass/state.sock"));
    }

    #[test]
    fn explicit_socket_paths_win() {
        let cfg = parse(
            r#"
            base_url = "http://hass.example:8123"
            token_file = "/run/secrets/hass_token"
            entity = "light.desk_lamp"
            command_socket = "/tmp/hass-light.sock"
            state_socket = "/tmp/hass-light-eww-output.sock"
            "#,
        );
        assert_eq!(cfg.command_socket(), PathBuf::from("/tmp/hass-light.sock"));
        assert_eq!(
            cfg.state_socket(),
            PathBuf::from("/tmp/hass-light-eww-output.sock")
        );
    }

    /// `services.hass.stepPercent = 5` in Nix writes `step_percent = 5`, a TOML
    /// *integer*, and nothing in the module can stop it: a float-valued option
    /// whose value happens to be whole comes out that way. So the percentages
    /// have to accept both spellings, and this is the test that says so.
    #[test]
    fn a_percentage_the_nix_module_wrote_as_an_integer_still_parses() {
        let cfg = parse(&format!(
            "{MINIMAL}\nstep_percent = 5\nmin_brightness_percent = 1"
        ));
        assert_eq!(cfg.step(), 5.0);
        assert_eq!(cfg.min_brightness_percent, 1.0);
    }

    #[test]
    fn a_zero_step_is_refused_rather_than_scrolling_nowhere() {
        let cfg = parse(&format!("{MINIMAL}\nstep_percent = 0.0"));
        assert!(cfg.step() > 0.0);
    }

    #[test]
    fn a_missing_token_says_which_two_places_it_looked() {
        let cfg = parse(
            r#"
            base_url = "http://hass.example:8123"
            entity = "light.desk_lamp"
            "#,
        );
        // Only meaningful with no HASS_TOKEN in the environment, which is the
        // case under `cargo test` unless someone exported one to run the daemon
        // by hand in the same shell.
        if std::env::var("HASS_TOKEN").is_err() {
            let err = cfg.token().unwrap_err().to_string();
            assert!(err.contains("token_file"), "{err}");
            assert!(err.contains("HASS_TOKEN"), "{err}");
        }
    }
}
