//! Everything the calendar window needs for one repaint, computed here so the
//! GUI is a renderer and nothing else.

use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};

use crate::calendar::Calendar;
use crate::model::{Day, DayKind};

/// The six-week grid every month is drawn on.
///
/// Always forty-two cells, never fewer: a month that needs five rows and one
/// that needs six would otherwise change the window's height as you page
/// through the year, which is exactly the kind of movement a calendar should
/// not have.
pub const CELLS: usize = 42;

pub struct MonthView {
    pub year: i32,
    pub month: u32,
    /// "agosto 2026", already in Spanish — the bar is in Spanish and so is the
    /// person reading it.
    pub title: String,
    /// Seven labels, rotated to the configured week start.
    pub weekday_labels: [&'static str; 7],
    pub cells: Vec<Cell>,
    /// This month's named non-working days, in date order. Rendered as a list
    /// under the grid: a colour says *that* a day is special, and only the name
    /// says which.
    pub holidays: Vec<Day>,
    /// No feriado data for this month's year — the grid is weekends-only.
    pub degraded: bool,
}

pub struct Cell {
    pub day: Day,
    /// False for the leading and trailing days borrowed from the neighbouring
    /// months.
    pub in_month: bool,
    pub is_today: bool,
    pub is_working: bool,
}

const MONTHS: [&str; 12] = [
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];

/// Monday-first, single letters. Two of them are "M" and two are "S"; that is
/// how a Spanish calendar reads and disambiguating them would look wrong.
const DOW: [&str; 7] = ["L", "M", "M", "J", "V", "S", "D"];

impl Calendar {
    /// Build the grid for one month.
    pub fn month_view(&self, year: i32, month: u32) -> MonthView {
        let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_else(|| {
            // Only reachable with a month outside 1..=12, which the callers
            // clamp; falling back to today beats a panic in a repaint.
            Local::now()
                .date_naive()
                .with_day(1)
                .expect("the 1st exists")
        });
        let today = Local::now().date_naive();
        let week_start = self.week().week_start;

        // How far back the grid starts: the first cell is the configured week
        // start on or before the 1st.
        let lead = (first.weekday().num_days_from_monday() as i64
            - week_start.num_days_from_monday() as i64)
            .rem_euclid(7);
        let grid_start = first - Duration::days(lead);

        let cells = (0..CELLS as i64)
            .filter_map(|i| grid_start.checked_add_signed(Duration::days(i)))
            .map(|date| Cell {
                in_month: date.month() == first.month() && date.year() == first.year(),
                is_today: date == today,
                is_working: self.is_working_day(date),
                day: self.day(date),
            })
            .collect::<Vec<_>>();

        let holidays = cells
            .iter()
            .filter(|c| c.in_month && c.day.name.is_some() && !c.is_working)
            .map(|c| c.day.clone())
            .collect();

        MonthView {
            year,
            month,
            title: format!("{} {year}", MONTHS[(month as usize - 1).min(11)]),
            weekday_labels: rotate(week_start),
            cells,
            holidays,
            degraded: !self.coverage().years.contains(&year),
        }
    }
}

fn rotate(week_start: Weekday) -> [&'static str; 7] {
    let shift = week_start.num_days_from_monday() as usize;
    let mut out = [""; 7];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = DOW[(i + shift) % 7];
    }
    out
}

impl MonthView {
    /// The month before this one.
    pub fn prev(&self) -> (i32, u32) {
        if self.month == 1 {
            (self.year - 1, 12)
        } else {
            (self.year, self.month - 1)
        }
    }

    /// The month after this one.
    pub fn next(&self) -> (i32, u32) {
        if self.month == 12 {
            (self.year + 1, 1)
        } else {
            (self.year, self.month + 1)
        }
    }
}

impl Cell {
    /// The CSS classes this cell wears, beyond the base `.cal-day`.
    ///
    /// Returned rather than applied so the caller can *replace* a label's
    /// classes wholesale — adding them would leave a stale `.holiday` behind
    /// the first time you page past a feriado.
    pub fn classes(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.in_month {
            out.push("other-month");
        }
        if self.is_today {
            out.push("today");
        }
        // `Working` and `ForcedWork` add nothing: an ordinary day should look
        // ordinary, and a forced-work day explains itself in the tooltip rather
        // than shouting in the grid.
        match self.day.kind {
            DayKind::Weekend => out.push("weekend"),
            DayKind::Holiday => out.push("holiday"),
            DayKind::Bridge => out.push("bridge"),
            DayKind::NonWorking => out.push("non-working"),
            DayKind::Working | DayKind::ForcedWork => {}
        }
        out
    }

    /// What to show on hover.
    pub fn tooltip(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(name) = &self.day.name {
            parts.push(name.clone());
        }
        if self.day.overridden {
            parts.push(match self.is_working {
                true => "se trabaja (definido a mano)".to_string(),
                false => "no se trabaja (definido a mano)".to_string(),
            });
        } else if self.day.kind == DayKind::Bridge {
            parts.push(match self.is_working {
                true => "puente turístico — se trabaja".to_string(),
                false => "puente turístico".to_string(),
            });
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}
