//! Data shapes shared between the daemon and its clients.
//!
//! [`UsageStatus`] is what the eww pill consumes (the same fields the old
//! `get-claude-usage` script emitted); [`Chart`], [`Bar`], and [`Session`]
//! are what the GUI renders (the same shapes the old `claude-usage-history`
//! script emitted). Keeping them identical lets the eww widget and the GUI
//! render the same thing the old scripts produced, byte for byte.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// One row of the `usage` table: a single reading from the endpoint at a
/// moment in time. `at` is a Unix timestamp (seconds); the utilization fields
/// are percentages (0.0–100.0); the `*_resets_at` are RFC 3339 timestamps.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageReading {
    pub at: i64,
    pub session: f64,
    pub weekly: f64,
    pub session_resets_at: Option<String>,
    pub weekly_resets_at: Option<String>,
}

/// What the eww pill reads. This is the exact shape of the old script's
/// `Output` struct, so the widget's `?.field` access keeps working unchanged.
///
/// `failed` is the bar's own fallback for the CLI not being able to reach the
/// daemon at all; `stale`/`expired` come from the daemon's last poll; and
/// `configured` is the only thing that hides the widget entirely (a machine
/// with no token refresh command configured).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageStatus {
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

/// A view of the chart the daemon can compute. `Day` is a calendar day (not
/// "last 24h"), so the day you land on from the month view is the same kind of
/// thing as the day you're already looking at.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum View {
    Day(NaiveDate),
    Month,
}

impl View {
    /// Today's day view — the most common request.
    pub fn today() -> Self {
        Self::Day(chrono::Local::now().date_naive())
    }
}

/// One bar in the chart: an hour (day view) or a day (month view).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bar {
    pub label: String,
    /// Percent of the limit burned during this bucket.
    pub value: f64,
    /// Height as a percentage of the tallest bar, for drawing.
    pub pct: i64,
    /// No readings landed in this bucket: the machine was off or the bar wasn't
    /// running. Drawn differently from a genuine zero.
    pub gap: bool,
    /// Which day this bar stands for, so the month view can be clicked into.
    /// Empty in the day view, where a bar is an hour already shown.
    pub date: String,
    pub tooltip: String,
}

/// One 5-hour window as a rectangle drawn behind the bars, measured in pixels
/// across the chart so it begins and ends where the window actually did.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    /// Idle time before this window opened, in pixels.
    pub gap: i64,
    pub width: i64,
    pub band: bool,
    /// Whether the window's own start and end fall inside the chart. A window
    /// that opened before the chart did, or that hasn't closed by the time the
    /// chart runs out, is cut off rather than finished — see [`Week`], which is
    /// drawn the same way.
    pub opens: bool,
    pub closes: bool,
    /// What this window cost, written in the strip below the axis.
    pub label: String,
    pub tooltip: String,
}

/// One 7-day window, the same idea as [`Session`] one scale up: a rectangle
/// drawn behind the session windows. A day usually sits inside a single one (two
/// if the reset falls in it), a month spans four or five. Weeks butt up against
/// each other with no idle time between them, so these carry an absolute left
/// edge instead of a session's gap-from-the-last-one.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Week {
    /// Left edge, in pixels from the left of the chart. Clipped to the chart,
    /// so a window that opened before it begins at 0.
    pub x: i64,
    pub width: i64,
    pub band: bool,
    /// Whether the window's own start and end fall inside the chart. The
    /// decorations that mark a span's ends — the border down the left edge, the
    /// rounded corners — are only drawn on the side where the window really
    /// begins or ends; a side that is merely where the chart stopped is left
    /// square and unmarked, so it reads as continuing past the edge.
    pub opens: bool,
    pub closes: bool,
    /// How much of the weekly limit this window spent in total — the whole
    /// window, not the part of it on screen, so stepping between days of the
    /// same week reads the same number. Written in the strip below the session
    /// labels.
    pub label: String,
    pub tooltip: String,
}

/// The whole chart, day or month. The `bar_width` × `bars.len()` product is the
/// `chart_width` every layer agrees on, so the weekly rectangles at the back,
/// the session rectangles over them, and the bars in front all line up
/// independently.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Chart {
    pub range: String,
    pub date: String,
    pub title: String,
    pub previous: String,
    pub next: String,
    pub total: f64,
    pub peak: f64,
    pub bar_width: i64,
    pub chart_width: i64,
    pub lead_px: i64,
    pub weeks: Vec<Week>,
    pub sessions: Vec<Session>,
    pub bars: Vec<Bar>,
}
