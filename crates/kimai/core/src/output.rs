//! The one line the bar reads, and the numbers behind it.
//!
//! Every derived value is computed here rather than in yuck. The precedence
//! between unreachable, feriado, running and idle is real logic — a feriado
//! expects zero hours, which changes what "behind" even means — and
//! reimplementing it in a widget expression is how the pill and the popup end
//! up disagreeing. So the CLI ships `state`, `label` and `tooltip` already
//! rendered, and the widget is a `(label :text {kimai?.label})`.

use chrono::{DateTime, Local, NaiveDate};
use holidays::DayKind;
use serde::{Deserialize, Serialize};

/// Which of the four things the widget is showing.
///
/// Ordered by precedence, which is also the order the pill decides in:
/// unreachable beats everything (there are no numbers to show), a feriado beats
/// running (the day expects nothing either way), and running beats idle.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Failed,
    Holiday,
    Running,
    Idle,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Failed => "failed",
            State::Holiday => "holiday",
            State::Running => "running",
            State::Idle => "idle",
        }
    }
}

/// What the bar reads.
///
/// `PartialEq` is what lets the emitter drop a reading identical to the last
/// one. Comparing the struct rather than its rendered line means the no-change
/// path never serialises at all, and it compares what the widget actually reads
/// rather than an encoding of it.
///
/// Field order is the JSON's field order, and the fallback test depends on it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    /// A config was found. False hides the widget entirely.
    pub configured: bool,
    /// The last poll answered.
    pub ok: bool,
    /// `!ok`. Kept as its own field because the widget reads both, and
    /// `!kimai?.ok` in yuck is one `?.` away from being silently wrong.
    pub failed: bool,
    pub state: String,

    pub running: bool,
    pub task: String,
    pub description: String,
    /// `HH:MM` the running entry began.
    pub since: String,
    pub running_time: String,

    pub today_time: String,
    pub week_time: String,
    pub month_time: String,
    pub today_seconds: i64,
    pub week_seconds: i64,
    pub month_seconds: i64,

    /// Expected through the end of today — today counts in full.
    pub expected_today: String,
    pub expected_week: String,
    pub expected_month: String,

    /// Signed and rendered: `+2:15`, `-3:39:49`. Positive means ahead.
    pub today_delta: String,
    pub week_delta: String,
    pub month_delta: String,
    pub today_delta_seconds: i64,
    pub week_delta_seconds: i64,
    pub month_delta_seconds: i64,

    /// 0–100, clamped. The ring around the pill's icon.
    pub day_percent: i64,
    pub week_percent: i64,
    pub month_percent: i64,

    pub holiday: bool,
    pub holiday_name: String,
    pub day_kind: String,
    /// No feriado data for a year in play: the arithmetic above used the
    /// weekend rule alone and is wrong by a day's target per holiday.
    pub holidays_degraded: bool,

    pub week_start: String,
    pub week_end: String,
    pub month_label: String,

    /// Pre-rendered, so the yuck is a label and the popup is a layout.
    pub label: String,
    pub tooltip: String,
    pub error: String,
}

impl Output {
    /// What the bar shows when there is nothing to show.
    ///
    /// Every field present and readable, because a widget full of `?.` accesses
    /// against a missing object renders as a row of empty strings that looks
    /// like a working zero.
    pub fn failed(error: &str) -> Self {
        let zero = format_duration(0);
        Self {
            configured: true,
            ok: false,
            failed: true,
            state: State::Failed.as_str().into(),
            running: false,
            task: String::new(),
            description: String::new(),
            since: String::new(),
            running_time: zero.clone(),
            today_time: zero.clone(),
            week_time: zero.clone(),
            month_time: zero.clone(),
            today_seconds: 0,
            week_seconds: 0,
            month_seconds: 0,
            expected_today: zero.clone(),
            expected_week: zero.clone(),
            expected_month: zero.clone(),
            today_delta: format_signed(0),
            week_delta: format_signed(0),
            month_delta: format_signed(0),
            today_delta_seconds: 0,
            week_delta_seconds: 0,
            month_delta_seconds: 0,
            day_percent: 0,
            week_percent: 0,
            month_percent: 0,
            holiday: false,
            holiday_name: String::new(),
            day_kind: DayKind::Working.as_str().into(),
            holidays_degraded: true,
            week_start: String::new(),
            week_end: String::new(),
            month_label: String::new(),
            label: "kimai unreachable".into(),
            tooltip: format!(
                "kimai: {}",
                if error.is_empty() { "no answer" } else { error }
            ),
            error: error.to_string(),
        }
    }

    /// What the bar shows on a machine with no Kimai configured at all.
    pub fn unconfigured() -> Self {
        Self {
            configured: false,
            ..Self::failed("no kimai config")
        }
    }
}

/// Everything one reading needs, before it is turned into an [`Output`].
pub struct Reading {
    pub now: DateTime<Local>,
    pub today: NaiveDate,
    pub today_seconds: i64,
    pub week_seconds: i64,
    pub month_seconds: i64,
    pub expected_today: i64,
    pub expected_week: i64,
    pub expected_month: i64,
    pub running: Option<RunningEntry>,
    pub day: holidays::Day,
    pub is_working_day: bool,
    pub week: (NaiveDate, NaiveDate),
    pub holidays_degraded: bool,
}

pub struct RunningEntry {
    pub task: String,
    pub description: String,
    pub since: String,
    pub seconds: i64,
}

impl Reading {
    pub fn output(&self) -> Output {
        let running = self.running.as_ref();
        let holiday = !self.is_working_day && self.day.kind != holidays::DayKind::Weekend;

        let state = if holiday {
            State::Holiday
        } else if running.is_some() {
            State::Running
        } else {
            State::Idle
        };

        let today_delta = self.today_seconds - self.expected_today;
        let week_delta = self.week_seconds - self.expected_week;
        let month_delta = self.month_seconds - self.expected_month;

        let out = Output {
            configured: true,
            ok: true,
            failed: false,
            state: state.as_str().into(),

            running: running.is_some(),
            task: running.map(|r| r.task.clone()).unwrap_or_default(),
            description: running.map(|r| r.description.clone()).unwrap_or_default(),
            since: running.map(|r| r.since.clone()).unwrap_or_default(),
            running_time: format_duration(running.map(|r| r.seconds).unwrap_or(0)),

            today_time: format_duration(self.today_seconds),
            week_time: format_duration(self.week_seconds),
            month_time: format_duration(self.month_seconds),
            today_seconds: self.today_seconds,
            week_seconds: self.week_seconds,
            month_seconds: self.month_seconds,

            expected_today: format_duration(self.expected_today),
            expected_week: format_duration(self.expected_week),
            expected_month: format_duration(self.expected_month),

            today_delta: format_signed(today_delta),
            week_delta: format_signed(week_delta),
            month_delta: format_signed(month_delta),
            today_delta_seconds: today_delta,
            week_delta_seconds: week_delta,
            month_delta_seconds: month_delta,

            day_percent: percent(self.today_seconds, self.expected_today),
            week_percent: percent(self.week_seconds, self.expected_week),
            month_percent: percent(self.month_seconds, self.expected_month),

            holiday,
            holiday_name: self.day.name.clone().unwrap_or_default(),
            day_kind: self.day.kind.as_str().into(),
            holidays_degraded: self.holidays_degraded,

            week_start: self.week.0.to_string(),
            week_end: self.week.1.to_string(),
            month_label: self.today.format("%Y-%m").to_string(),

            label: String::new(),
            tooltip: String::new(),
            error: String::new(),
        };

        Output {
            label: self.label(state, &out),
            tooltip: self.tooltip(state, &out),
            ..out
        }
    }

    /// The collapsed pill: what is on the clock now, and what today comes to.
    ///
    /// Totals rather than a delta. The pill is a reading, not a judgement — how
    /// far ahead or behind the week is depends on a target, and a signed number
    /// against an unstated target is the kind of thing you end up re-deriving in
    /// your head every time you glance at it. Both deltas are still a hover
    /// away in the tooltip and laid out in full in `kimai-gui`, which is where
    /// the comparison belongs.
    fn label(&self, state: State, out: &Output) -> String {
        match state {
            State::Failed => "kimai unreachable".into(),
            State::Holiday if self.today_seconds == 0 => "feriado".into(),
            State::Holiday => format!("feriado · {}", out.today_time),
            // Session first, then the day it is adding to.
            State::Running => format!("{} / {}", out.running_time, out.today_time),
            // Nothing running, so there is no session to put in front of it.
            State::Idle => out.today_time.clone(),
        }
    }

    fn tooltip(&self, state: State, out: &Output) -> String {
        let first = match state {
            State::Failed => return format!("kimai: {}", out.error),
            State::Holiday => match &self.day.name {
                Some(name) => format!("feriado: {name}"),
                None => "no se trabaja hoy".to_string(),
            },
            State::Running => {
                let mut line = if out.task.is_empty() {
                    "sin proyecto".to_string()
                } else {
                    out.task.clone()
                };
                if !out.description.is_empty() {
                    line.push_str(&format!(" — {}", out.description));
                }
                line.push_str(&format!(" · desde {}", out.since));
                line
            }
            State::Idle => "nada corriendo".to_string(),
        };

        let today = if self.expected_today == 0 {
            format!("hoy   {} / feriado ({})", out.today_time, out.today_delta)
        } else {
            format!(
                "hoy   {} / {} ({})",
                out.today_time, out.expected_today, out.today_delta
            )
        };

        let rest = format!(
            "sem   {} / {} ({})  ·  mes  {} / {} ({})",
            out.week_time,
            out.expected_week,
            out.week_delta,
            out.month_time,
            out.expected_month,
            out.month_delta
        );

        let mut lines = vec![first, today, rest];
        if self.holidays_degraded {
            lines.push("sin datos de feriados — sólo fines de semana".into());
        }
        lines.join("\n")
    }
}

/// A percentage that survives a zero target.
///
/// On a feriado nothing is expected, so the honest reading of "how much of
/// today's target is done" is either "all of it" or "none of it" — and a naive
/// `secs * 100 / target` would divide by zero and take the process with it.
/// This is not hypothetical: it happens roughly nineteen days a year.
pub fn percent(seconds: i64, target: i64) -> i64 {
    if target <= 0 {
        return if seconds > 0 { 100 } else { 0 };
    }
    (seconds * 100 / target).clamp(0, 100)
}

/// clockin's format, kept because the bar has read this way since before Kimai:
/// the largest unit leads and the smaller ones are padded behind it, so a timer
/// under an hour is not padded out to a leading zero it never needed.
pub fn format_duration(total: i64) -> String {
    let total = total.max(0);
    let hours = total / 3600;
    let mins = total % 3600 / 60;
    let secs = total % 60;

    if hours > 0 {
        format!("{hours}:{mins:02}:{secs:02}")
    } else if mins > 0 {
        format!("{mins}:{secs:02}")
    } else {
        format!("{secs}s")
    }
}

/// The same, with a sign that is always shown.
///
/// Always, including on zero, and including the `+`: a bare `2:15` in a pill
/// that sometimes shows debt is ambiguous exactly when it matters. ASCII `-`
/// rather than a minus sign, because the bar's font renders one of them at the
/// wrong width.
pub fn format_signed(total: i64) -> String {
    let sign = if total < 0 { '-' } else { '+' };
    format!("{sign}{}", format_duration(total.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_lead_with_the_largest_unit_they_need() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(125), "2:05");
        assert_eq!(format_duration(3725), "1:02:05");
    }

    #[test]
    fn a_delta_always_carries_its_sign() {
        assert_eq!(format_signed(0), "+0s");
        assert_eq!(format_signed(8100), "+2:15:00");
        assert_eq!(format_signed(-13189), "-3:39:49");
    }

    #[test]
    fn a_feriado_does_not_divide_by_zero() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(4800, 0), 100);
        assert_eq!(percent(14400, 28800), 50);
        // Overtime fills the ring rather than overflowing it.
        assert_eq!(percent(36000, 28800), 100);
    }

    /// A reading with two hours booked and, when `running`, forty minutes of it
    /// still on the clock. Expected is set well above both so that any delta
    /// leaking back into the pill would be an obvious signed number.
    fn reading(running: bool, working_day: bool) -> Reading {
        let now = Local::now();
        Reading {
            now,
            today: now.date_naive(),
            today_seconds: 7200,
            week_seconds: 7200,
            month_seconds: 7200,
            expected_today: 28800,
            expected_week: 144_000,
            expected_month: 576_000,
            running: running.then(|| RunningEntry {
                task: "SmartMachine / Firmware".into(),
                description: String::new(),
                since: "09:00".into(),
                seconds: 2400,
            }),
            day: holidays::Day {
                date: now.date_naive(),
                kind: if working_day {
                    holidays::DayKind::Working
                } else {
                    holidays::DayKind::Holiday
                },
                name: (!working_day).then(|| "Día de prueba".to_string()),
                overridden: false,
            },
            is_working_day: working_day,
            week: (now.date_naive(), now.date_naive()),
            holidays_degraded: false,
        }
    }

    /// The pill reports totals, never a delta — the whole point of the last
    /// change to `label`. Asserting on the exact strings is the only way that
    /// stays true, since the deltas are still right there in the same `Output`.
    #[test]
    fn the_pill_shows_the_session_and_the_day_not_a_delta() {
        let running = reading(true, true).output();
        assert_eq!(running.label, "40:00 / 2:00:00");

        let idle = reading(false, true).output();
        assert_eq!(idle.label, "2:00:00");

        let feriado = reading(true, false).output();
        assert_eq!(feriado.label, "feriado · 2:00:00");

        // The deltas are still computed and published; they are just not on the
        // pill. The tooltip and kimai-gui read these.
        assert_eq!(running.today_delta, "-6:00:00");
        assert!(!running.label.contains(&running.today_delta));
        assert!(!running.label.contains(&running.week_delta));
    }

    /// A feriado with nothing booked has no total worth showing, so it says only
    /// what the day is.
    #[test]
    fn an_empty_feriado_says_only_that() {
        let mut r = reading(false, false);
        r.today_seconds = 0;
        assert_eq!(r.output().label, "feriado");
    }
}
