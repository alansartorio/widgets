//! The on-disk store and the chart computation.
//!
//! The schema is one table of usage readings; the chart logic buckets those
//! readings into hourly (day) or daily (month) bars and reconstructs the
//! 5-hour session windows behind them, plus the 7-day windows behind those. The
//! bars and the sessions are ported from the old `claude-usage-history`
//! rust-script, so the numbers match what the eww popup showed.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Timelike};
use rusqlite::Connection;

use crate::models::{Bar, Chart, Session, UsageReading, View, Week};

/// How long the two kinds of window run for. A reset stamp minus one of these
/// is when that window opened.
const SESSION_LENGTH: Duration = Duration::hours(5);
const WEEK_LENGTH: Duration = Duration::days(7);
/// The endpoint reports the same reset with sub-second jitter, and occasionally
/// a minute either side of it. Two starts closer together than this are the
/// same window.
const JITTER: Duration = Duration::minutes(15);

/// When the window that resets at `resets_at` opened, rounded to the minute so
/// the jitter in the stamp doesn't turn one window into several.
fn window_start(resets_at: &str, length: Duration) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(resets_at)
        .ok()
        .map(|r| r.with_timezone(&Local) - length)
        .and_then(|s| s.with_second(0)?.with_nanosecond(0))
}

/// The 7-day windows that overlap `from`..`to`, each with the share of the
/// weekly limit it ended up spending.
///
/// This asks the table on its own rather than reading the chart's own readings,
/// because a week is longer than any chart: the week a Wednesday belongs to
/// opened the previous Saturday and closes on the next one, so that Wednesday's
/// readings only ever show part of it. Totalling what was in view is what made
/// the same week read differently from each of its days. The caller reaches a
/// full week past the chart at both ends, the way the reading query reaches a
/// full session past it for the 5-hour windows.
///
/// Taking the maximum is what makes the figure whole-window: the weekly
/// percentage is a running total inside a window, not a rate, so its largest
/// reading is where the window ended up (and, for the week still open, where it
/// stands now). SQLite groups the readings by their reset stamp to the minute —
/// the same rounding [`window_start`] does — which collapses thousands of
/// readings to one row per window before anything crosses into Rust; the
/// 15-minute merge then happens here, as it does for sessions.
fn weekly_windows(
    db: &Connection,
    from: DateTime<Local>,
    to: DateTime<Local>,
) -> Result<Vec<(DateTime<Local>, f64)>> {
    let mut query = db.prepare(
        "SELECT min(weekly_resets_at), max(weekly)
         FROM usage
         WHERE at >= ?1 AND at < ?2 AND weekly_resets_at IS NOT NULL
         GROUP BY substr(weekly_resets_at, 1, 16)",
    )?;
    let mut windows: Vec<(DateTime<Local>, f64)> = query
        .query_map([from.timestamp(), to.timestamp()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|(stamp, used)| window_start(&stamp, WEEK_LENGTH).map(|start| (start, used)))
        .collect();
    windows.sort_by_key(|(start, _)| *start);

    let mut merged: Vec<(DateTime<Local>, f64)> = Vec::new();
    for (start, used) in windows {
        match merged.last_mut() {
            Some((known, most)) if start - *known < JITTER => *most = most.max(used),
            _ => merged.push((start, used)),
        }
    }
    Ok(merged)
}

/// Create the schema if it doesn't exist.
pub fn init(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage (
            at                INTEGER PRIMARY KEY,
            session           REAL NOT NULL,
            weekly            REAL NOT NULL,
            session_resets_at TEXT,
            weekly_resets_at  TEXT
        );",
    )?;
    Ok(())
}

/// Open the database at `path`, creating the schema on first use.
pub fn open(path: &std::path::Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let db = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    init(&db)?;
    Ok(db)
}

/// Append a reading. `INSERT OR REPLACE` on the `at` primary key means a
/// re-poll within the same second overwrites the first, which is harmless.
pub fn record(db: &Connection, reading: &UsageReading) -> Result<()> {
    db.execute(
        "INSERT OR REPLACE INTO usage (at, session, weekly, session_resets_at, weekly_resets_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            reading.at,
            reading.session,
            reading.weekly,
            reading.session_resets_at,
            reading.weekly_resets_at,
        ],
    )?;
    Ok(())
}

/// Compute the chart for a day or the last 30 days. This is a direct port of
/// the old `claude-usage-history` script's algorithm.
pub fn query_chart(db: &Connection, view: &View) -> Result<Chart> {
    let today = Local::now().date_naive();
    let (day, buckets, bucket): (Option<NaiveDate>, i64, Duration) = match view {
        View::Day(d) => (Some(*d), 24, Duration::hours(1)),
        View::Month => (None, 30, Duration::days(1)),
    };

    let chart_start = match day {
        Some(day) => midnight(day),
        None => midnight(today) - Duration::days(buckets - 1),
    };
    let chart_end = chart_start + bucket * (buckets as i32);

    let mut burned = vec![0.0_f64; buckets as usize];
    let mut seen = vec![false; buckets as usize];

    // Reach 5h past the chart at both ends so a window straddling the boundary
    // is totalled correctly from either side, and the first bucket has an
    // earlier reading to measure its rise against.
    let mut query = db.prepare(
        "SELECT at, session, session_resets_at
         FROM usage WHERE at >= ?1 AND at < ?2 ORDER BY at",
    )?;
    let readings: Vec<(i64, f64, Option<String>)> = query
        .query_map(
            [
                (chart_start - SESSION_LENGTH).timestamp(),
                (chart_end + SESSION_LENGTH).timestamp(),
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;

    // Dedup window starts: the endpoint returns the same reset with sub-second
    // jitter, so round to the minute and merge anything within 15 minutes.
    let window_of = |resets_at: &str| window_start(resets_at, SESSION_LENGTH);
    let mut window_starts: Vec<DateTime<Local>> = readings
        .iter()
        .filter_map(|(_, _, r)| r.as_deref())
        .filter_map(window_of)
        .collect();
    window_starts.sort();
    window_starts.dedup();
    window_starts.dedup_by(|later, earlier| *later - *earlier < JITTER);

    let mut window_burn = vec![0.0_f64; window_starts.len().max(1)];
    let mut previous: Option<f64> = None;
    for (at, session, resets_at) in &readings {
        let rise = previous
            .filter(|before| session > before)
            .map(|before| session - before);

        if let (Some(rise), Some(window)) = (
            rise,
            resets_at
                .as_deref()
                .and_then(window_of)
                .map(|start| window_starts.partition_point(|known| *known <= start))
                .filter(|idx| *idx > 0),
        ) {
            window_burn[window - 1] += rise;
        }

        if let Some(slot) = DateTime::from_timestamp(*at, 0)
            .map(|t| t.with_timezone(&Local))
            .map(|t| (t - chart_start).num_seconds() / bucket.num_seconds())
            .filter(|slot| (0..buckets).contains(slot))
        {
            seen[slot as usize] = true;
            if let Some(rise) = rise {
                burned[slot as usize] += rise;
            }
        }
        previous = Some(*session);
    }

    // Weekly windows come from their own query, not from `readings`: see
    // [`weekly_windows`].
    let weekly = weekly_windows(db, chart_start - WEEK_LENGTH, chart_end + WEEK_LENGTH)?;

    let peak = burned.iter().copied().fold(0.0_f64, f64::max);
    let bars: Vec<Bar> = (0..buckets)
        .map(|slot| {
            let start = chart_start + bucket * (slot as i32);
            let value = burned[slot as usize];
            let gap = !seen[slot as usize];
            let (label, when) = match day {
                Some(_) => (
                    start.format("%H").to_string(),
                    start.format("%H:00").to_string(),
                ),
                None => (
                    start.format("%d").to_string(),
                    start.format("%a %d %b").to_string(),
                ),
            };
            Bar {
                tooltip: if gap {
                    format!("{when} · —")
                } else {
                    format!("{when} · {value:.1}%")
                },
                label,
                value: (value * 10.0).round() / 10.0,
                pct: if peak > 0.0 {
                    (value / peak * 100.0).round() as i64
                } else {
                    0
                },
                gap,
                date: match day {
                    Some(_) => String::new(),
                    None => start.format("%Y-%m-%d").to_string(),
                },
            }
        })
        .collect();

    // Sessions are laid out in pixels (not buckets): a window opening at 16:50
    // begins five sixths of the way through the 16:00 bar. Only the day view has
    // them; the month view is just bars.
    let bar_width: i64 = if day.is_some() { 21 } else { 17 };
    let chart_width = bar_width * buckets;
    let span = (chart_end - chart_start).num_seconds() as f64;
    let across = |at: DateTime<Local>| -> f64 {
        ((at - chart_start).num_seconds() as f64 / span * chart_width as f64)
            .clamp(0.0, chart_width as f64)
    };

    let mut sessions: Vec<Session> = Vec::new();
    let mut lead_px = 0;
    if day.is_some() {
        let mut cursor = 0.0_f64;
        for (index, start) in window_starts.iter().enumerate() {
            let end = *start + SESSION_LENGTH;
            if end <= chart_start || *start >= chart_end {
                continue;
            }
            let left = across(*start);
            let right = across(end);
            if sessions.is_empty() {
                lead_px = left.round() as i64;
                cursor = left;
            }
            let gap = (left - cursor).round().max(0.0) as i64;
            let width = (right - left).round().max(1.0) as i64;
            let burned_here = window_burn.get(index).copied().unwrap_or(0.0);
            sessions.push(Session {
                gap,
                width,
                band: index % 2 == 1,
                opens: *start >= chart_start,
                closes: end <= chart_end,
                label: if width >= 30 {
                    format!("{burned_here:.0}%")
                } else {
                    String::new()
                },
                tooltip: format!(
                    "{} → {} · {burned_here:.0}%",
                    start.format("%H:%M"),
                    end.format("%H:%M")
                ),
            });
            cursor = right;
        }
    }

    // Weekly windows, laid out the same way but by absolute left edge, and in
    // both views: a day sits inside one (two, if the reset falls in it), a month
    // spans four or five. Both edges are clamped to the chart by `across`, so a
    // window that opened before it starts at 0 and one still open runs to the
    // right edge; `opens`/`closes` are what tell the two apart from an edge that
    // is the window's own.
    let weeks: Vec<Week> = weekly
        .iter()
        .filter_map(|(start, used)| {
            let end = *start + WEEK_LENGTH;
            if end <= chart_start || *start >= chart_end {
                return None;
            }
            let left = across(*start);
            let width = (across(end) - left).round().max(1.0) as i64;
            Some(Week {
                x: left.round() as i64,
                width,
                opens: *start >= chart_start,
                closes: end <= chart_end,
                // Which shade this window gets is decided by the window itself,
                // not by its place in the list: consecutive weeks are exactly
                // 7 days apart, so this alternates, and it alternates the same
                // way whichever day or month the window is being shown under.
                band: (start.timestamp() / WEEK_LENGTH.num_seconds()).rem_euclid(2) == 1,
                // Named while there's room for the word, so the strip's second
                // row of percentages says what it is measuring.
                label: match width {
                    w if w >= 56 => format!("week {used:.0}%"),
                    w if w >= 28 => format!("{used:.0}%"),
                    _ => String::new(),
                },
                tooltip: format!(
                    "{} → {} · {used:.0}% of the weekly limit",
                    start.format("%a %d %b %H:%M"),
                    end.format("%a %d %b %H:%M")
                ),
            })
        })
        .collect();

    Ok(Chart {
        range: if day.is_some() { "day" } else { "month" }.to_string(),
        date: day.map(|d| d.to_string()).unwrap_or_default(),
        title: match day {
            Some(d) if d == today => "today".to_string(),
            Some(d) => midnight(d).format("%a %d %b").to_string(),
            None => "30 days".to_string(),
        },
        previous: day
            .map(|d| (d - Duration::days(1)).to_string())
            .unwrap_or_default(),
        next: day
            .filter(|d| *d < today)
            .map(|d| (d + Duration::days(1)).to_string())
            .unwrap_or_default(),
        total: (burned.iter().sum::<f64>() * 10.0).round() / 10.0,
        peak: (peak * 10.0).round() / 10.0,
        bar_width,
        chart_width,
        lead_px,
        weeks,
        sessions,
        bars,
    })
}

fn midnight(day: NaiveDate) -> DateTime<Local> {
    Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight exists"))
        .earliest()
        .expect("midnight falls somewhere in the day")
}
