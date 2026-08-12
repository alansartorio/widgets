//! The per-year cache on disk.
//!
//! In the *state* directory, not the cache one, and that is not pedantry: this
//! data cannot be regenerated offline. A laptop off the VPN that lost
//! `~/.cache` would silently drop every feriado from the month and start
//! reporting eight extra hours of debt per holiday — with no error, because a
//! missing file reads exactly like a year with nothing in it. State is the
//! honest home for "regenerable only with the network".

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::source::ApiDay;

/// One year, as stored.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YearFile {
    pub year: i32,
    pub fetched_at: DateTime<Local>,
    /// Recorded so a cache written by a future version that changed source can
    /// be recognised rather than trusted.
    pub source: String,
    pub days: Vec<ApiDay>,
}

pub fn path(state_dir: &Path, year: i32) -> PathBuf {
    state_dir.join(format!("ar-{year}.json"))
}

/// Read a year, or `None`.
///
/// Never an error: an absent file is the normal state before the first fetch,
/// and a corrupt one is something to overwrite rather than something to make a
/// caller handle. Both mean "no data for this year", which is exactly what
/// [`Coverage::degraded`](crate::Coverage) exists to report.
pub fn load(state_dir: &Path, year: i32) -> Option<YearFile> {
    let raw = std::fs::read_to_string(path(state_dir, year)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Write a year, atomically.
///
/// Temp file plus rename, because the alternative is a process killed mid-write
/// leaving a truncated file that parses as garbage — and [`load`] would then
/// treat a *present* cache as an empty year, which is the one failure mode with
/// no visible symptom.
pub fn store(state_dir: &Path, file: &YearFile) -> Result<()> {
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("creating {}", state_dir.display()))?;

    let final_path = path(state_dir, file.year);
    let tmp_path = final_path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(file)?;

    std::fs::write(&tmp_path, body).with_context(|| format!("writing {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &final_path)
        .with_context(|| format!("renaming into {}", final_path.display()))?;
    Ok(())
}

/// Has this year's copy aged past `max_age`?
///
/// A future `fetched_at` — a clock that jumped, a file copied from another
/// machine — counts as fresh rather than as a negative age.
pub fn is_stale(file: &YearFile, max_age: chrono::Duration, now: DateTime<Local>) -> bool {
    now.signed_duration_since(file.fetched_at) > max_age
}
