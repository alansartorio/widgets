//! Asking Kimai as little as possible while still reading live.
//!
//! Three rates, and they are three because they answer different questions.
//!
//! * The **fast** poll (60 s) asks about today: the running entry, and what is
//!   already booked. That is the only part of the picture that changes while
//!   you watch it.
//! * The **span** poll (900 s) asks about the union of this week and this month
//!   — the union, because a week straddles the month boundary and two windows
//!   would be two requests — and buckets the answer by date. Yesterday does not
//!   change; asking about it every minute is a request for a number that is
//!   already right.
//! * The **tick** (1 s) is not a request at all. It counts the running entry
//!   forward locally, because a clock that only moved on the poll would sit
//!   still for a minute at a time.
//!
//! The trick that keeps week and month as fresh as today, at one request a
//! minute: the fast poll's total *replaces today's bucket*. So the month is the
//! span's buckets for every day but this one, plus today's live figure.
//!
//! A fourth rate hides in here too — the rate anything actually changes. With a
//! timer running that is the tick; idle it is the poll; with the server away it
//! is never. [`Emitter`](crate::emitter::Emitter) is what turns that into how
//! often the bar is woken.

use std::collections::BTreeMap;
use std::time::Instant;

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use holidays::Calendar;
use kimai_api::{Client, Entry, Query};

use crate::config::Config;
use crate::output::{Output, Reading, RunningEntry};

/// The running entry, as far as the poller cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub id: i64,
    /// Clamped to midnight: an entry that began yesterday and is still running
    /// would otherwise pour the whole night into today's total.
    pub counts_from: DateTime<Local>,
    /// The real start, as you would read it off a clock.
    pub since: String,
    pub task: String,
    pub description: String,
}

pub struct Poller {
    client: Client,
    cfg: Config,
    cal: Calendar,

    /// Finished seconds per day, from the span poll. Today's entry is
    /// overwritten by every fast poll.
    buckets: BTreeMap<NaiveDate, i64>,
    running: Option<Running>,

    today: NaiveDate,
    last_fast: Option<Instant>,
    last_span: Option<Instant>,
    /// The span the buckets currently cover, so a date rollover can be noticed.
    span: Option<(NaiveDate, NaiveDate)>,
}

impl Poller {
    pub fn new(cfg: Config) -> Result<Self> {
        let cal = Calendar::open(cfg.work_week());
        Ok(Self {
            client: cfg.client()?,
            cal,
            buckets: BTreeMap::new(),
            running: None,
            today: Local::now().date_naive(),
            last_fast: None,
            last_span: None,
            span: None,
            cfg,
        })
    }

    pub fn calendar(&self) -> &Calendar {
        &self.cal
    }

    pub fn calendar_mut(&mut self) -> &mut Calendar {
        &mut self.cal
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Do whatever this moment calls for, and hand back the reading.
    ///
    /// Blocking, and only when it actually needs the network — most calls do
    /// nothing but arithmetic and return in microseconds, which is what makes
    /// it safe to call at tick rate.
    pub fn tick(&mut self, now: DateTime<Local>) -> Result<Output> {
        let today = now.date_naive();

        // A rollover invalidates everything: the buckets cover the wrong span,
        // today's bucket belongs to yesterday, and the running entry's midnight
        // clamp has moved.
        if today != self.today {
            self.today = today;
            self.span = None;
            self.last_span = None;
            self.last_fast = None;
        }

        let wanted_span = self.wanted_span(today);
        let span_due = match (self.last_span, self.span) {
            (None, _) | (_, None) => true,
            (Some(at), Some(span)) => {
                span != wanted_span || at.elapsed() >= self.cfg.span_poll_interval()
            }
        };

        if span_due {
            self.poll_span(wanted_span)?;
            self.last_span = Some(Instant::now());
        }

        let fast_due = self
            .last_fast
            .is_none_or(|at| at.elapsed() >= self.cfg.poll_interval());
        if fast_due {
            self.poll_today(today)?;
            self.last_fast = Some(Instant::now());
            // The override file is re-read on the same beat, so a feriado
            // marked by hand takes effect within one poll rather than at the
            // next restart. It is a small file and this is once a minute.
            self.cal.reload_overrides();
        }

        Ok(self.reading(now).output())
    }

    /// The reading as of `now`, with no network at all — the tick.
    pub fn extrapolate(&self, now: DateTime<Local>) -> Output {
        self.reading(now).output()
    }

    /// This week and this month, unioned.
    fn wanted_span(&self, today: NaiveDate) -> (NaiveDate, NaiveDate) {
        let week = self.cal.week_of(today);
        let month = self.cal.month_of(today);
        (
            (*week.start()).min(*month.start()),
            (*week.end()).max(*month.end()),
        )
    }

    fn poll_span(&mut self, span: (NaiveDate, NaiveDate)) -> Result<()> {
        let entries = self.client.timesheets_all(&Query::span(span.0, span.1))?;

        self.buckets = bucket_by_day(&entries);
        self.span = Some(span);
        Ok(())
    }

    fn poll_today(&mut self, today: NaiveDate) -> Result<()> {
        let active = self.client.active()?;
        let entries = self.client.timesheets_all(&Query::day(today))?;

        // Replaces rather than adds: this is the same day the span poll already
        // counted, and the fresher of the two answers wins outright.
        let finished = kimai_api::finished_seconds(&entries);
        self.buckets.insert(today, finished);

        // Kimai returns the newest first; with several clocks running at once
        // the bar can only show one, and the one just started is the one being
        // watched.
        self.running = match active.first() {
            None => None,
            Some(entry) => Some(running_from(entry, midnight(today))?),
        };
        Ok(())
    }

    fn reading(&self, now: DateTime<Local>) -> Reading {
        let today = self.today;

        let running_secs = self
            .running
            .as_ref()
            .map(|r| (now - r.counts_from).num_seconds().max(0))
            .unwrap_or(0);

        // Today's bucket holds only the *finished* entries; the running one is
        // counted forward here so it lands in the total once, live.
        let sum = |from: NaiveDate, to: NaiveDate| -> i64 {
            let booked: i64 = self.buckets.range(from..=to).map(|(_, secs)| *secs).sum();
            if (from..=to).contains(&today) {
                booked + running_secs
            } else {
                booked
            }
        };

        let week = self.cal.week_of(today);
        let month = self.cal.month_of(today);

        Reading {
            now,
            today,
            today_seconds: sum(today, today),
            week_seconds: sum(*week.start(), *week.end()),
            month_seconds: sum(*month.start(), *month.end()),

            expected_today: self
                .cal
                .expected_through(today..=today, today)
                .num_seconds(),
            expected_week: self.cal.expected_through(week.clone(), today).num_seconds(),
            expected_month: self
                .cal
                .expected_through(month.clone(), today)
                .num_seconds(),

            running: self.running.as_ref().map(|r| RunningEntry {
                task: r.task.clone(),
                description: r.description.clone(),
                since: r.since.clone(),
                seconds: running_secs,
            }),

            day: self.cal.day(today),
            is_working_day: self.cal.is_working_day(today),
            week: (*week.start(), *week.end()),
            holidays_degraded: self.cal.coverage().degraded,
        }
    }
}

/// Group finished entries by the day they began.
///
/// By `begin` rather than by `end`, so an entry that ran past midnight lands on
/// the day you would say you worked it. It disagrees with Kimai's own filtering
/// at the boundary by at most one entry, and agrees with how a person reads
/// their week.
fn bucket_by_day(entries: &[Entry]) -> BTreeMap<NaiveDate, i64> {
    let mut out = BTreeMap::new();
    for entry in entries {
        if entry.end.is_none() {
            continue;
        }
        let Ok(begin) = entry.started() else { continue };
        *out.entry(begin.date_naive()).or_insert(0) += entry.duration.unwrap_or(0);
    }
    out
}

fn running_from(entry: &Entry, floor: DateTime<Local>) -> Result<Running> {
    let begin = entry.started()?;
    Ok(Running {
        id: entry.id,
        counts_from: begin.max(floor),
        since: begin.format("%H:%M").to_string(),
        task: entry.task(),
        description: entry.description.clone().unwrap_or_default(),
    })
}

/// The first instant of a local day.
///
/// A DST jump that skips midnight leaves no local 00:00 to name; the first
/// instant of the day that does exist is the honest floor.
fn midnight(date: NaiveDate) -> DateTime<Local> {
    Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight always exists"))
        .earliest()
        .unwrap_or_else(|| {
            Local
                .from_local_datetime(&date.and_hms_opt(1, 0, 0).expect("01:00 always exists"))
                .earliest()
                .unwrap_or_else(Local::now)
        })
}

/// How stale the feriado cache may be before a background refresh is worth
/// doing — exposed so the CLI and the GUI schedule it the same way.
pub fn holidays_refresh_due(cal: &Calendar, max_age: Duration) -> bool {
    let now = Local::now();
    cal.coverage().degraded
        || cal
            .coverage()
            .fetched_at
            .is_none_or(|at| now.signed_duration_since(at) > max_age)
        || (now.month() >= 11 && !cal.coverage().years.contains(&(now.year() + 1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, begin: &str, end: Option<&str>, duration: i64) -> Entry {
        let end = match end {
            Some(e) => format!(r#", "end": "{e}""#),
            None => String::new(),
        };
        serde_json::from_str(&format!(
            r#"{{"id": {id}, "begin": "{begin}"{end}, "duration": {duration}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn entries_bucket_by_the_day_they_began() {
        let entries = vec![
            entry(
                1,
                "2026-08-10T09:00:00-0300",
                Some("2026-08-10T12:00:00-0300"),
                10800,
            ),
            entry(
                2,
                "2026-08-10T13:00:00-0300",
                Some("2026-08-10T17:00:00-0300"),
                14400,
            ),
            entry(
                3,
                "2026-08-11T09:00:00-0300",
                Some("2026-08-11T11:00:00-0300"),
                7200,
            ),
            // Still running: not booked anywhere, it is counted forward instead.
            entry(4, "2026-08-11T14:00:00-0300", None, 0),
        ];

        let buckets = bucket_by_day(&entries);
        assert_eq!(buckets[&"2026-08-10".parse().unwrap()], 25200);
        assert_eq!(buckets[&"2026-08-11".parse().unwrap()], 7200);
        assert_eq!(buckets.len(), 2);
    }

    #[test]
    fn an_overnight_entry_counts_from_midnight_not_from_when_it_started() {
        let yesterday = entry(1, "2026-08-10T22:00:00-0300", None, 0);
        let floor = midnight("2026-08-11".parse().unwrap());
        let running = running_from(&yesterday, floor).unwrap();

        assert_eq!(running.counts_from, floor);
        // The label still says when it really began, which is what you would
        // read off a clock.
        assert_eq!(running.since, "22:00");
    }

    #[test]
    fn a_normal_entry_counts_from_when_it_started() {
        let today = entry(1, "2026-08-11T09:30:00-0300", None, 0);
        let floor = midnight("2026-08-11".parse().unwrap());
        let running = running_from(&today, floor).unwrap();

        assert!(running.counts_from > floor);
        assert_eq!(running.since, "09:30");
    }
}
