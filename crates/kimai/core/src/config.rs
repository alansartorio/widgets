//! What the CLI and the GUI both need to know before they can ask Kimai
//! anything.
//!
//! A file rather than arguments, which is a change from the binary this
//! replaces: that one took the base URL on the command line and the token file
//! from an environment variable the eww yuck had to set. Two processes need
//! these now — `kimai tail` and `kimai-gui`, spawned from different clicks —
//! and threading the same pair through two yuck strings is how they drift.
//!
//! The token itself never appears here, only its *path*. sops decrypts it into
//! `/run/secrets` at activation and this reads it there on every start, so
//! nothing secret lands in `~/.config` or in the nix store.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Duration, Weekday};
use holidays::WorkWeek;
use serde::Deserialize;

pub const FILE_NAME: &str = "config.toml";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// e.g. `https://kimai.example.com`.
    pub base_url: String,
    /// A file holding a Kimai personal API token. Read at start, not baked in.
    pub token_file: PathBuf,

    #[serde(default = "default_daily_target")]
    pub daily_target_hours: f64,
    #[serde(default = "default_week_start")]
    pub week_start: String,
    #[serde(default = "default_workdays")]
    pub workdays: Vec<String>,
    #[serde(default)]
    pub bridges_are_working: bool,

    /// How often today's numbers are refreshed.
    #[serde(default = "default_poll")]
    pub poll_seconds: u64,
    /// How often the surrounding week-and-month span is refreshed. Much rarer:
    /// yesterday does not change.
    #[serde(default = "default_span_poll")]
    pub span_poll_seconds: u64,
    /// How old the feriado cache may get before a refresh is attempted.
    #[serde(default = "default_holidays_max_age")]
    pub holidays_max_age_days: i64,
}

fn default_daily_target() -> f64 {
    8.0
}
fn default_week_start() -> String {
    "monday".into()
}
fn default_workdays() -> Vec<String> {
    ["mon", "tue", "wed", "thu", "fri"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}
fn default_poll() -> u64 {
    60
}
fn default_span_poll() -> u64 {
    900
}
fn default_holidays_max_age() -> i64 {
    7
}

impl Config {
    /// Every place a config could be, in the order they win.
    ///
    /// `$XDG_CONFIG_DIRS` is in there for the NixOS flavour of the module,
    /// which drops the file in `/etc/xdg/kimai` on a machine where the service
    /// is system-wide rather than per-user.
    pub fn search_paths() -> Vec<PathBuf> {
        if let Ok(explicit) = std::env::var("KIMAI_CONFIG")
            && !explicit.is_empty()
        {
            return vec![PathBuf::from(explicit)];
        }

        let mut out = vec![widgets_paths::Paths::user("kimai").config(FILE_NAME)];
        if let Ok(dirs) = std::env::var("XDG_CONFIG_DIRS") {
            out.extend(
                dirs.split(':')
                    .filter(|d| !d.is_empty())
                    .map(|d| Path::new(d).join("kimai").join(FILE_NAME)),
            );
        } else {
            out.push(PathBuf::from("/etc/xdg/kimai").join(FILE_NAME));
        }
        out
    }

    /// Load the first config that exists.
    ///
    /// `Ok(None)` means there is none, which is not an error: a machine without
    /// Kimai should have no widget, and the CLI says `configured: false` rather
    /// than exiting non-zero at the bar. A config that exists but does not parse
    /// *is* an error — that one is a typo somebody needs to see.
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

    /// Let the environment win, for running it by hand against a scratch
    /// instance without editing the file home-manager owns.
    pub fn with_env(mut self) -> Self {
        if let Ok(url) = std::env::var("KIMAI_URL")
            && !url.is_empty()
        {
            self.base_url = url;
        }
        if let Ok(file) = std::env::var("KIMAI_TOKEN_FILE")
            && !file.is_empty()
        {
            self.token_file = PathBuf::from(file);
        }
        self
    }

    /// Read the token off disk.
    ///
    /// Deliberately not cached across a process's life beyond the caller
    /// holding it: `kimai tail` reads it once at start and a rotated token
    /// means restarting the bar, which is the same bargain the previous binary
    /// made.
    pub fn token(&self) -> Result<String> {
        let raw = std::fs::read_to_string(&self.token_file)
            .with_context(|| format!("reading the token at {}", self.token_file.display()))?;
        Ok(raw.trim().to_string())
    }

    pub fn client(&self) -> Result<kimai_api::Client> {
        Ok(kimai_api::Client::new(&self.base_url, self.token()?))
    }

    /// The working week this config describes.
    ///
    /// An unreadable weekday name is dropped rather than fatal: a typo in
    /// `workdays` should cost you that day, visibly, not the whole widget.
    pub fn work_week(&self) -> WorkWeek {
        let mut workdays = [false; 7];
        for name in &self.workdays {
            if let Some(day) = parse_weekday(name) {
                workdays[day.num_days_from_monday() as usize] = true;
            }
        }
        // An empty or entirely misspelled list would mean nothing is ever
        // expected, and a month target of zero looks like a working feature.
        if workdays == [false; 7] {
            workdays = [true, true, true, true, true, false, false];
        }

        WorkWeek {
            week_start: parse_weekday(&self.week_start).unwrap_or(Weekday::Mon),
            workdays,
            daily_target: Duration::seconds((self.daily_target_hours * 3600.0).round() as i64),
            bridges_are_working: self.bridges_are_working,
        }
    }

    pub fn poll_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.poll_seconds.max(5))
    }

    pub fn span_poll_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.span_poll_seconds.max(self.poll_seconds.max(5)))
    }

    pub fn holidays_max_age(&self) -> Duration {
        Duration::days(self.holidays_max_age_days.max(1))
    }
}

fn parse_weekday(raw: &str) -> Option<Weekday> {
    Some(match raw.to_ascii_lowercase().as_str() {
        "mon" | "monday" | "lun" | "lunes" => Weekday::Mon,
        "tue" | "tuesday" | "mar" | "martes" => Weekday::Tue,
        "wed" | "wednesday" | "mie" | "miércoles" | "miercoles" => Weekday::Wed,
        "thu" | "thursday" | "jue" | "jueves" => Weekday::Thu,
        "fri" | "friday" | "vie" | "viernes" => Weekday::Fri,
        "sat" | "saturday" | "sab" | "sábado" | "sabado" => Weekday::Sat,
        "sun" | "sunday" | "dom" | "domingo" => Weekday::Sun,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Config {
        toml_edit::de::from_str(raw).unwrap()
    }

    #[test]
    fn the_defaults_are_a_normal_argentine_week() {
        let cfg = parse(
            r#"
            base_url = "https://kimai.example"
            token_file = "/run/secrets/kimai_token"
            "#,
        );
        let week = cfg.work_week();
        assert_eq!(week.week_start, Weekday::Mon);
        assert_eq!(week.daily_target, Duration::hours(8));
        assert_eq!(week.workdays, [true, true, true, true, true, false, false]);
        assert!(!week.bridges_are_working);
    }

    #[test]
    fn workdays_accept_spanish_and_a_half_day_target() {
        let cfg = parse(
            r#"
            base_url = "https://kimai.example"
            token_file = "/run/secrets/kimai_token"
            daily_target_hours = 6.5
            workdays = ["lunes", "mar", "miércoles"]
            "#,
        );
        let week = cfg.work_week();
        assert_eq!(
            week.workdays,
            [true, true, true, false, false, false, false]
        );
        assert_eq!(week.daily_target, Duration::minutes(390));
    }

    #[test]
    fn an_entirely_misspelled_week_falls_back_rather_than_expecting_nothing() {
        let cfg = parse(
            r#"
            base_url = "https://kimai.example"
            token_file = "/run/secrets/kimai_token"
            workdays = ["lunez", "marte"]
            "#,
        );
        assert_eq!(
            cfg.work_week().workdays,
            [true, true, true, true, true, false, false]
        );
    }

    #[test]
    fn a_span_poll_can_never_be_faster_than_the_fast_one() {
        let cfg = parse(
            r#"
            base_url = "https://kimai.example"
            token_file = "/run/secrets/kimai_token"
            poll_seconds = 300
            span_poll_seconds = 60
            "#,
        );
        assert_eq!(cfg.span_poll_interval(), cfg.poll_interval());
    }
}
