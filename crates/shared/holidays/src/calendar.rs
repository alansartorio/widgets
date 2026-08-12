//! The thing both consumers actually hold: a calendar you can ask about a day,
//! a month, or how many hours a range was supposed to be.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use anyhow::Result;
use chrono::{Datelike, Duration, Local, NaiveDate};

use crate::cache::{self, YearFile};
use crate::model::{Coverage, Day, DayKind, WorkWeek};
use crate::overrides::{self, Overrides};
use crate::source;

/// What a refresh did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refresh {
    /// The cache was already recent enough.
    Fresh,
    /// New data landed.
    Updated,
    /// The fetch failed. The cache stays in use and the message is on
    /// [`Coverage::last_error`] for a tooltip.
    Failed(String),
}

/// Where the files live. Split out so tests can point it at a scratch dir
/// without touching the real one.
#[derive(Clone, Debug)]
pub struct Locations {
    /// Holds `ar-<year>.json`.
    pub state_dir: PathBuf,
    /// The user's `overrides.toml`.
    pub overrides: PathBuf,
}

impl Locations {
    /// `$XDG_STATE_HOME/holidays` and `$XDG_CONFIG_HOME/holidays/overrides.toml`,
    /// with the usual `HOLIDAYS_*_DIR` overrides.
    pub fn from_env() -> Self {
        let paths = widgets_paths::Paths::user("holidays");
        Self {
            state_dir: paths.data_dir,
            overrides: std::env::var("HOLIDAYS_OVERRIDES")
                .ok()
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| paths.config_dir.join("overrides.toml")),
        }
    }
}

/// Feriados, overrides and the working-day arithmetic on top of both.
pub struct Calendar {
    locations: Locations,
    week: WorkWeek,
    years: BTreeMap<i32, YearFile>,
    /// Per year, the dates it names. Built once on load so `day()` is a lookup
    /// rather than a scan — the calendar GUI calls it forty-two times a repaint.
    index: BTreeMap<NaiveDate, (DayKind, String)>,
    overrides: Overrides,
    coverage: Coverage,
}

impl Calendar {
    /// Read whatever is on disk.
    ///
    /// Never touches the network and never fails. A missing cache is not an
    /// error a caller should have to handle at construction — it is a fact
    /// about the answers, reported through [`coverage`](Self::coverage), and
    /// the arithmetic degrades to the weekend rule rather than refusing to run.
    /// A bar that cannot draw its pill because a JSON file is absent is worse
    /// than a bar that draws it slightly wrong and says so.
    pub fn open(week: WorkWeek) -> Self {
        Self::open_at(Locations::from_env(), week)
    }

    pub fn open_at(locations: Locations, week: WorkWeek) -> Self {
        let mut cal = Self {
            locations,
            week,
            years: BTreeMap::new(),
            index: BTreeMap::new(),
            overrides: Overrides::new(),
            coverage: Coverage::default(),
        };
        cal.reload_overrides();
        cal.ensure_year(Local::now().year());
        cal
    }

    /// Re-read the override file.
    ///
    /// Cheap, and called on every poll on purpose: the whole point of that file
    /// is that editing it takes effect without a restart.
    pub fn reload_overrides(&mut self) {
        match overrides::load(&self.locations.overrides) {
            Ok(o) => self.overrides = o,
            Err(e) => {
                // A typo in a hand-edited file must not take the widget down.
                // Keep whatever was loaded before and say so.
                self.coverage.last_error = Some(format!("{}: {e:#}", self.locations.overrides.display()));
            }
        }
    }

    /// Load a year from disk if it is not already in memory.
    pub fn ensure_year(&mut self, year: i32) {
        if self.years.contains_key(&year) {
            return;
        }
        match cache::load(&self.locations.state_dir, year) {
            Some(file) => self.absorb(file),
            None => {
                self.coverage.degraded = true;
            }
        }
    }

    /// Fetch this year — and, once it is close enough to matter, next year too
    /// — if the cache is missing or older than `max_age`.
    ///
    /// Blocking. Call it off whatever thread is drawing.
    pub fn refresh_if_stale(&mut self, max_age: Duration) -> Refresh {
        let now = Local::now();
        let mut wanted = vec![now.year()];
        // From November the next month's arithmetic starts crossing into
        // January, and a laptop that goes off the VPN over the holidays would
        // otherwise spend all of January degraded.
        if now.month() >= 11 {
            wanted.push(now.year() + 1);
        }

        let mut result = Refresh::Fresh;
        for year in wanted {
            let stale = match self.years.get(&year) {
                None => true,
                Some(file) => cache::is_stale(file, max_age, now),
            };
            if !stale {
                continue;
            }
            match self.refresh_year(year) {
                Ok(()) => result = Refresh::Updated,
                Err(e) => {
                    let msg = format!("{e:#}");
                    self.coverage.last_error = Some(msg.clone());
                    // A failed fetch for next year, in November, is not worth
                    // reporting over a successful one for this year.
                    if result != Refresh::Updated {
                        result = Refresh::Failed(msg);
                    }
                }
            }
        }
        result
    }

    /// Fetch one year unconditionally and write it to the cache.
    pub fn refresh_year(&mut self, year: i32) -> Result<()> {
        let days = source::fetch_year(&source::agent(), year)?;
        let file = YearFile {
            year,
            fetched_at: Local::now(),
            source: source::ENDPOINT.to_string(),
            days,
        };
        cache::store(&self.locations.state_dir, &file)?;
        self.absorb(file);
        self.coverage.last_error = None;
        Ok(())
    }

    /// Index one year's worth of entries and record that we have it.
    fn absorb(&mut self, file: YearFile) {
        for entry in &file.days {
            let Ok(date) = entry.fecha.parse::<NaiveDate>() else {
                continue;
            };
            let kind = match entry.tipo.as_str() {
                "puente" => DayKind::Bridge,
                // inamovible, trasladable, and anything the API starts sending
                // that this does not know about. Treating an unknown category
                // as a feriado is the safe direction: a new one is far more
                // likely to be a day off than a day of work.
                _ => DayKind::Holiday,
            };
            self.index.insert(date, (kind, entry.nombre.clone()));
        }
        self.coverage.fetched_at = Some(file.fetched_at);
        let year = file.year;
        self.years.insert(year, file);
        if !self.coverage.years.contains(&year) {
            self.coverage.years.push(year);
            self.coverage.years.sort_unstable();
        }
        self.coverage.degraded = false;
    }

    /// What one date counts as.
    ///
    /// Precedence, in order: the override file, then the API, then the weekend
    /// rule. The override winning outright is the whole reason it exists — a
    /// `work` on a feriado has to beat the feriado, or you could never book the
    /// day you actually worked.
    pub fn day(&self, date: NaiveDate) -> Day {
        if let Some(over) = self.overrides.get(&date) {
            return Day {
                date,
                kind: over.kind,
                name: over
                    .name
                    .clone()
                    .or_else(|| self.index.get(&date).map(|(_, n)| n.clone())),
                overridden: true,
            };
        }

        if let Some((kind, name)) = self.index.get(&date) {
            return Day {
                date,
                kind: *kind,
                name: Some(name.clone()),
                overridden: false,
            };
        }

        Day {
            date,
            kind: if self.week.is_workday(date.weekday()) {
                DayKind::Working
            } else {
                DayKind::Weekend
            },
            name: None,
            overridden: false,
        }
    }

    /// Does this date carry the daily target?
    ///
    /// [`day`](Self::day) has already resolved the precedence, so this only has
    /// to read the kind — with one guard on top. A puente that lands on a
    /// Saturday is still a Saturday, so `bridges_are_working` must not drag a
    /// weekend into the count. `ForcedWork` gets no such guard, deliberately:
    /// writing `kind = "work"` against a Sunday is an explicit statement about
    /// that Sunday, and second-guessing it would make the override file a
    /// suggestion.
    pub fn is_working_day(&self, date: NaiveDate) -> bool {
        match self.day(date).kind {
            DayKind::ForcedWork => true,
            DayKind::Bridge => self.week.bridges_are_working && self.week.is_workday(date.weekday()),
            other => other.is_working(&self.week),
        }
    }

    pub fn days(&self, range: RangeInclusive<NaiveDate>) -> impl Iterator<Item = Day> + '_ {
        let (start, end) = (*range.start(), *range.end());
        start
            .iter_days()
            .take_while(move |d| *d <= end)
            .map(|d| self.day(d))
    }

    pub fn working_days(&self, range: RangeInclusive<NaiveDate>) -> u32 {
        let (start, end) = (*range.start(), *range.end());
        start
            .iter_days()
            .take_while(|d| *d <= end)
            .filter(|d| self.is_working_day(*d))
            .count() as u32
    }

    /// What the whole range was supposed to be.
    pub fn expected(&self, range: RangeInclusive<NaiveDate>) -> Duration {
        self.week.daily_target * self.working_days(range) as i32
    }

    /// What the range was supposed to be *so far* — up to and including
    /// `today`.
    ///
    /// Including today is the decision the whole ahead/behind reading rests on:
    /// you start each working morning eight hours behind and reach zero when
    /// the day is done, so the number doubles as a to-do rather than a verdict
    /// on yesterday.
    pub fn expected_through(
        &self,
        range: RangeInclusive<NaiveDate>,
        today: NaiveDate,
    ) -> Duration {
        let end = (*range.end()).min(today);
        if end < *range.start() {
            return Duration::zero();
        }
        self.expected(*range.start()..=end)
    }

    /// The week `date` falls in, honouring [`WorkWeek::week_start`].
    pub fn week_of(&self, date: NaiveDate) -> RangeInclusive<NaiveDate> {
        let offset = (date.weekday().num_days_from_monday() as i64
            - self.week.week_start.num_days_from_monday() as i64)
            .rem_euclid(7);
        let start = date - Duration::days(offset);
        start..=start + Duration::days(6)
    }

    /// The calendar month `date` falls in.
    pub fn month_of(&self, date: NaiveDate) -> RangeInclusive<NaiveDate> {
        let start = date.with_day(1).expect("the 1st always exists");
        let next = if start.month() == 12 {
            NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)
        }
        .expect("the 1st of the next month always exists");
        start..=next.pred_opt().expect("a month has a last day")
    }

    pub fn coverage(&self) -> &Coverage {
        &self.coverage
    }

    pub fn week(&self) -> &WorkWeek {
        &self.week
    }

    pub fn overrides(&self) -> &Overrides {
        &self.overrides
    }

    pub fn locations(&self) -> &Locations {
        &self.locations
    }

    /// Add or replace an override, on disk and in memory.
    pub fn set_override(
        &mut self,
        date: NaiveDate,
        kind: DayKind,
        name: Option<&str>,
    ) -> Result<()> {
        overrides::set(&self.locations.overrides, date, kind, name)?;
        self.reload_overrides();
        Ok(())
    }

    /// Drop an override, on disk and in memory.
    pub fn unset_override(&mut self, date: NaiveDate) -> Result<()> {
        overrides::unset(&self.locations.overrides, date)?;
        self.reload_overrides();
        Ok(())
    }
}
