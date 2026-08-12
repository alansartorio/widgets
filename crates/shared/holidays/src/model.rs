//! What a day is, and what a working week is.

use chrono::{Duration, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

/// What a date counts as.
///
/// Argentina has more than two answers here, which is the whole reason this
/// crate exists. `Holiday` and `Weekend` are the easy ones; the other three are
/// not:
///
/// * `Bridge` — a *puente turístico*, declared by decree a year or two ahead.
///   Whether it is worked is the employer's call, hence
///   [`WorkWeek::bridges_are_working`].
/// * `NonWorking` — a *día no laborable*, like Jueves Santo or the religious
///   holidays. Same idea, decided per date rather than per category, and absent
///   from the API entirely — these only ever come from the override file.
/// * `ForcedWork` — a date the API calls a holiday and the override file says
///   is worked anyway. Kept distinct from `Working` so a calendar can say
///   *why* a red day is being counted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DayKind {
    Working,
    Weekend,
    Holiday,
    Bridge,
    NonWorking,
    ForcedWork,
}

impl DayKind {
    /// Does this day carry the daily target?
    pub fn is_working(self, week: &WorkWeek) -> bool {
        match self {
            DayKind::Working | DayKind::ForcedWork => true,
            DayKind::Bridge => week.bridges_are_working,
            DayKind::Weekend | DayKind::Holiday | DayKind::NonWorking => false,
        }
    }

    /// The spelling used in JSON and as a CSS class, so the CLI, the GUI and
    /// the calendar's stylesheet cannot drift apart on it.
    pub fn as_str(self) -> &'static str {
        match self {
            DayKind::Working => "working",
            DayKind::Weekend => "weekend",
            DayKind::Holiday => "holiday",
            DayKind::Bridge => "bridge",
            DayKind::NonWorking => "non-working",
            DayKind::ForcedWork => "forced-work",
        }
    }

    /// Parse the same spelling back — what the override file's `kind =` accepts.
    ///
    /// `"work"` is allowed as an alias for `forced-work` because that is what a
    /// person writing the file means: "this one I work".
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "working" => DayKind::Working,
            "weekend" => DayKind::Weekend,
            "holiday" | "feriado" => DayKind::Holiday,
            "bridge" | "puente" => DayKind::Bridge,
            "non-working" | "no-laborable" => DayKind::NonWorking,
            "forced-work" | "work" => DayKind::ForcedWork,
            _ => return None,
        })
    }
}

/// One date, decided.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Day {
    pub date: NaiveDate,
    pub kind: DayKind,
    /// "Día del Trabajador". Absent on an ordinary day.
    pub name: Option<String>,
    /// The override file had an opinion about this date. Worth showing in a
    /// tooltip: a red day that is being counted looks like a bug otherwise.
    pub overridden: bool,
}

impl Day {
    pub fn is_working(&self, week: &WorkWeek) -> bool {
        self.kind.is_working(week)
    }
}

/// The shape of a working week: which days count, for how long, and what to do
/// with the ambiguous ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkWeek {
    /// Which day a week starts on. Monday here; the calendar grid reads it too.
    pub week_start: Weekday,
    /// Indexed by [`Weekday::num_days_from_monday`].
    pub workdays: [bool; 7],
    pub daily_target: Duration,
    /// Whether a *puente turístico* is worked. False by default: the whole
    /// point of the category is that it is not a feriado unless someone says so.
    pub bridges_are_working: bool,
}

impl Default for WorkWeek {
    fn default() -> Self {
        Self {
            week_start: Weekday::Mon,
            workdays: [true, true, true, true, true, false, false],
            daily_target: Duration::hours(8),
            bridges_are_working: false,
        }
    }
}

impl WorkWeek {
    /// Is this weekday one of the working ones, ignoring holidays entirely?
    pub fn is_workday(&self, weekday: Weekday) -> bool {
        self.workdays[weekday.num_days_from_monday() as usize]
    }
}

/// How much holiday data is actually on hand.
///
/// `degraded` is the field callers must not ignore: it means a year in play has
/// no data at all, so every answer came from the weekend rule alone. A month
/// total computed that way is wrong by eight hours per feriado, and the widget
/// showing it should say so rather than look confident.
#[derive(Clone, Debug, Default)]
pub struct Coverage {
    pub years: Vec<i32>,
    pub degraded: bool,
    pub last_error: Option<String>,
    pub fetched_at: Option<chrono::DateTime<chrono::Local>>,
}
