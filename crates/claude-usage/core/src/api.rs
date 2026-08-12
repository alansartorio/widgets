//! Polling the Anthropic usage endpoint and refreshing the token.
//!
//! The daemon is the only caller. The token comes from one place: the
//! credentials file in the data dir, refreshed by running the command in
//! `CLAUDE_USAGE_REFRESH_COMMAND` (set by the NixOS module to whatever the
//! user configured — typically an ssh that cats a remote credentials file).
//! There is no fallback to the user's home and no baked-in ssh host: those are
//! machine-specific and belong in the local config, not the shared module.
//!
//! Ported from the old `get-claude-usage` rust-script, minus the home-directory
//! fallbacks that don't apply to a dedicated service user.

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Local};
use serde::Serialize;
use std::fs;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ureq::Agent;

use crate::models::UsageReading;

const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Don't try the refresh command more than once per 5 minutes, so an
/// unreachable token host doesn't mean an ssh on every poll.
const REFRESH_MIN_INTERVAL: u64 = 5 * 60;
const REFRESH_COMMAND_ENV: &str = "CLAUDE_USAGE_REFRESH_COMMAND";

/// What [`poll`] returns: the raw usage value from the endpoint (for the DB),
/// plus the status flags the pill displays.
pub struct PollOutcome {
    /// The reading to append to the database. `None` if the endpoint didn't
    /// answer and we have no fresh data to record.
    pub reading: Option<UsageReading>,
    /// The status to show on the pill.
    pub status: UsageStatusSnapshot,
}

/// The pill's data, in the same shape as the old script's `Output` struct so
/// the eww widget reads it unchanged. This is the in-memory twin of
/// [`crate::models::UsageStatus`]; the daemon keeps one around to serve
/// [`crate::proto::Request::Status`] without re-reading the DB.
#[derive(Clone, Debug, Default, Serialize)]
pub struct UsageStatusSnapshot {
    pub ok: bool,
    pub stale: bool,
    pub expired: bool,
    pub failed: bool,
    pub configured: bool,
    pub fetched_at: String,
    pub status: u32,
    pub session: i64,
    pub weekly: i64,
    pub session_reset: String,
    pub weekly_reset: String,
}

impl From<UsageStatusSnapshot> for crate::models::UsageStatus {
    fn from(s: UsageStatusSnapshot) -> Self {
        crate::models::UsageStatus {
            ok: s.ok,
            stale: s.stale,
            expired: s.expired,
            failed: s.failed,
            configured: s.configured,
            fetched_at: s.fetched_at,
            status: s.status,
            session: s.session,
            weekly: s.weekly,
            session_reset: s.session_reset,
            weekly_reset: s.weekly_reset,
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is before the epoch")
        .as_secs()
}

fn age_of(path: &std::path::Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(now_secs().saturating_sub(modified.duration_since(UNIX_EPOCH).ok()?.as_secs()))
}

/// ISO 8601 in, local time out. Empty string if the parse fails, so the pill
/// renders nothing rather than crashing the widget expression.
fn format_time(stamp: Option<&str>, format: &str) -> String {
    stamp
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Local).format(format).to_string())
        .unwrap_or_default()
}

/// Read the persisted credentials and extract a live access token.
///
/// The credentials file holds either a bare access token or a copy of a
/// machine's `~/.claude/.credentials.json`. A credentials JSON carries its
/// own expiry, so a dead token is spotted before asking the endpoint.
fn read_token(credentials_path: &std::path::Path) -> Option<String> {
    let text = fs::read_to_string(credentials_path).ok()?;
    let text = text.trim();
    if !text.starts_with('{') {
        return Some(text.split_whitespace().collect());
    }
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    let oauth = &parsed["claudeAiOauth"];
    if let Some(expires_at) = oauth["expiresAt"].as_i64()
        && expires_at / 1000 < now_secs() as i64
    {
        return None;
    }
    oauth["accessToken"].as_str().map(str::to_string)
}

/// Whether a refresh command is configured at all. The pill hides entirely
/// when this is false (no credentials to work with on this machine).
pub fn is_configured() -> bool {
    std::env::var(REFRESH_COMMAND_ENV)
        .ok()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

/// Run the configured refresh command and persist whatever it prints.
///
/// Throttled to once per [`REFRESH_MIN_INTERVAL`] unless `force` is set, so a
/// dead token host doesn't cause an ssh on every poll. The returned
/// credentials are validated before overwriting the existing file — only a
/// live token replaces a live token.
fn refresh_token(
    credentials_path: &std::path::Path,
    stamp_path: &std::path::Path,
    force: bool,
) -> Result<()> {
    if !force
        && let Some(age) = age_of(stamp_path)
        && age < REFRESH_MIN_INTERVAL
    {
        return Err(anyhow!("last refresh attempt was {age}s ago"));
    }
    if let Some(parent) = stamp_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(stamp_path, "")?;

    let cmd = std::env::var(REFRESH_COMMAND_ENV).context("refresh command not set")?;
    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .output()
        .context("running refresh command")?;
    if !output.status.success() {
        return Err(anyhow!(
            "refresh command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let fetched = String::from_utf8(output.stdout)?;
    let fetched = fetched.trim().to_string();
    // Validate before overwriting: only a live token replaces a working file.
    let _token = token_from(&fetched).context("refresh output was not a live token")?;

    if let Some(parent) = credentials_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(credentials_path, &fetched)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(credentials_path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// A bare access token, or the one inside a credentials file.
fn token_from(credentials: &str) -> Option<String> {
    let credentials = credentials.trim();
    if !credentials.starts_with('{') {
        return Some(credentials.split_whitespace().collect());
    }
    let parsed: serde_json::Value = serde_json::from_str(credentials).ok()?;
    let oauth = &parsed["claudeAiOauth"];
    if let Some(expires_at) = oauth["expiresAt"].as_i64()
        && expires_at / 1000 < now_secs() as i64
    {
        return None;
    }
    oauth["accessToken"].as_str().map(str::to_string)
}

struct Answer {
    status: u32,
    body: String,
}

fn ask_endpoint(token: &str) -> Result<Answer> {
    let agent: Agent = Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        // 401 and 403 are answers we care about, not transport failures.
        .http_status_as_error(false)
        .build()
        .into();

    let mut response = agent
        .get(ENDPOINT)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .call()?;

    Ok(Answer {
        status: response.status().as_u16() as u32,
        body: response.body_mut().read_to_string()?,
    })
}

/// Poll the endpoint once. Handles the token refresh-and-retry, the cache
/// fallback, and building the status snapshot.
///
/// `force_refresh` skips the refresh throttle. The daemon passes `false`
/// except on the very first poll after start (so a freshly-installed service
/// with an expired seed token doesn't wait 5 minutes to fetch one).
pub fn poll(
    credentials_path: &std::path::Path,
    stamp_path: &std::path::Path,
    force_refresh: bool,
) -> PollOutcome {
    let configured = is_configured() || credentials_path.is_file();

    // Whatever goes wrong, the pill still gets an answer.
    let token = read_token(credentials_path);
    let mut answer = match &token {
        Some(token) => ask_endpoint(token)
            .map_err(|e| tracing::warn!("request failed: {e:#}"))
            .ok(),
        None => None,
    };

    // A dead token, or one the endpoint turns away, is worth a refresh and one
    // retry before giving up on this poll.
    let rejected = answer
        .as_ref()
        .map(|a| a.status == 401 || a.status == 403)
        .unwrap_or(true);
    if rejected
        && refresh_token(credentials_path, stamp_path, force_refresh)
            .map_err(|e| tracing::warn!("token refresh: {e:#}"))
            .is_ok()
        && let Some(token) = read_token(credentials_path)
    {
        answer = ask_endpoint(&token)
            .map_err(|e| tracing::warn!("request after refresh failed: {e:#}"))
            .ok();
    }

    let answer = match answer {
        Some(a) => a,
        None => {
            return PollOutcome {
                reading: None,
                status: expired_or_unreachable(configured, token.is_some()),
            };
        }
    };

    match answer.status {
        200 => {}
        401 | 403 => {
            return PollOutcome {
                reading: None,
                status: UsageStatusSnapshot {
                    expired: true,
                    configured,
                    ..Default::default()
                },
            };
        }
        status => {
            return PollOutcome {
                reading: None,
                status: UsageStatusSnapshot {
                    configured,
                    status,
                    ..Default::default()
                },
            };
        }
    }

    let usage: serde_json::Value = match serde_json::from_str(&answer.body) {
        Ok(u) => u,
        Err(_) => {
            return PollOutcome {
                reading: None,
                status: UsageStatusSnapshot {
                    configured,
                    status: answer.status,
                    ..Default::default()
                },
            };
        }
    };

    let reading = UsageReading {
        at: now_secs() as i64,
        session: usage["five_hour"]["utilization"].as_f64().unwrap_or(0.0),
        weekly: usage["seven_day"]["utilization"].as_f64().unwrap_or(0.0),
        session_resets_at: usage["five_hour"]["resets_at"].as_str().map(str::to_string),
        weekly_resets_at: usage["seven_day"]["resets_at"].as_str().map(str::to_string),
    };

    let status = UsageStatusSnapshot {
        ok: true,
        configured: true,
        fetched_at: Local::now().format("%H:%M").to_string(),
        status: 200,
        session: reading.session.round() as i64,
        weekly: reading.weekly.round() as i64,
        session_reset: format_time(reading.session_resets_at.as_deref(), "%H:%M"),
        weekly_reset: format_time(reading.weekly_resets_at.as_deref(), "%b %d"),
        ..Default::default()
    };

    PollOutcome {
        reading: Some(reading),
        status,
    }
}

/// The status when there's nothing to show for this poll.
///
/// Having no live token is the expired case — the credentials file is missing,
/// or the one in it has passed its `expiresAt` — and the pill says so. A token
/// we hold but couldn't spend, because the request itself never completed, is
/// not expired: that's a network or endpoint problem, which the pill words as
/// "no answer". Telling the two apart matters, since the fixes differ.
fn expired_or_unreachable(configured: bool, had_token: bool) -> UsageStatusSnapshot {
    UsageStatusSnapshot {
        expired: !had_token,
        configured,
        ..Default::default()
    }
}
