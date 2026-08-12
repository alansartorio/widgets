//! A client for Kimai's REST API — the two endpoints the bar needs.
//!
//! Extracted from the single binary this integration used to be, unchanged in
//! behaviour. Everything here was learned against the real instance, and most
//! of it is not guessable:
//!
//! * Kimai 2.x wants a personal API token as `Authorization: Bearer <token>`
//!   (profile → API access). The old `X-AUTH-USER`/`X-AUTH-TOKEN` pair is gone.
//! * Its timestamps are ISO 8601 with the offset *unseparated* —
//!   `2026-08-07T15:55:38-0300` — which is not RFC 3339, so the strict parser
//!   alone rejects every timestamp it sends.
//! * The collection endpoint returns the *running* record too, at duration 0,
//!   despite being filtered on `end`. Anything summing durations has to drop
//!   the entries with no end or it will count the running one twice: once at
//!   zero here, once counted forward by the caller.
//! * `project` and `activity` come back as bare integer ids from the collection
//!   and as whole objects from `/timesheets/active` — same key, two types. See
//!   [`Relation`].
//!
//! ureq with rustls rather than openssl, because linking the system copy bakes
//! a store path into the binary and the next rebuild moves it out from under a
//! process the bar keeps running for days.

use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Local, NaiveDate, NaiveDateTime};
use serde::Deserialize;

/// Well under any poll interval, so a hung request cannot stall the next one.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Entries per page. The API defaults to 50; a month of a full day's work is
/// comfortably under this, and a caller still pages in case it is not.
pub const PAGE_SIZE: u32 = 100;

/// A ceiling on paging, so a misread response cannot spin forever. Twenty pages
/// of 100 is two thousand entries — years of work, not a month of it.
pub const MAX_PAGES: u32 = 20;

/// One timesheet record, from either endpoint.
///
/// `project` and `activity` are the interesting part: see [`Relation`]. Both
/// are optional in the sense that the relation itself can be null, and the
/// label is built from whichever parts have a name.
#[derive(Clone, Debug, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub id: i64,
    pub begin: String,
    #[serde(default)]
    pub end: Option<String>,
    #[serde(default)]
    pub duration: Option<i64>,
    #[serde(default)]
    pub project: Option<Relation>,
    #[serde(default)]
    pub activity: Option<Relation>,
    #[serde(default)]
    pub description: Option<String>,
}

/// A reference to a project or an activity.
///
/// Kimai sends these under the same key in two different *types*, chosen by
/// serialization group rather than by field name. `Timesheet.php` declares both
/// against the same `activity`/`project` key:
///
/// * `Not_Expanded` — a virtual property emitting the relation's id as a bare
///   integer: `"activity": 667`. This is what the collection endpoint sends.
/// * `Expanded` — the whole related object, which is what `/timesheets/active`
///   sends, and the only form that carries a name.
///
/// So a type that expects the object fails on the collection with `invalid type:
/// integer, expected struct`, and the id form is not an error to be tolerated —
/// it is what that endpoint always sends. Only the name is ever wanted, so the
/// id deserializes to a relation without one.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Relation {
    /// `Not_Expanded`: the bare id.
    Id(i64),
    /// `Expanded`: the object. `name` is optional so an unfamiliar shape
    /// degrades to a nameless relation rather than failing the whole response —
    /// serde reports an untagged mismatch as "did not match any variant", which
    /// names neither the field nor the endpoint that sent it.
    Object {
        #[serde(default)]
        name: Option<String>,
    },
}

impl Relation {
    /// The display name, when this came back expanded.
    pub fn name(&self) -> Option<&str> {
        match self {
            Relation::Id(_) => None,
            Relation::Object { name } => name.as_deref(),
        }
    }
}

impl Entry {
    /// No end means the clock is still running on it.
    pub fn is_running(&self) -> bool {
        self.end.is_none()
    }

    /// `"Project / Activity"`, skipping whichever half has no name — which is
    /// either half that is null, and both halves on a collection response.
    pub fn task(&self) -> String {
        [&self.project, &self.activity]
            .iter()
            .filter_map(|part| part.as_ref().and_then(Relation::name))
            .collect::<Vec<_>>()
            .join(" / ")
    }

    pub fn started(&self) -> Result<DateTime<Local>> {
        parse_timestamp(&self.begin)
    }
}

/// A window of time to ask about.
#[derive(Clone, Debug)]
pub struct Query {
    pub begin: NaiveDateTime,
    pub end: NaiveDateTime,
    pub page: u32,
    pub size: u32,
}

impl Query {
    /// One whole day, local time.
    pub fn day(date: NaiveDate) -> Self {
        Self::span(date, date)
    }

    /// A span of whole days, inclusive at both ends.
    pub fn span(from: NaiveDate, to: NaiveDate) -> Self {
        Self {
            begin: from.and_hms_opt(0, 0, 0).expect("midnight always exists"),
            // 23:59:59 rather than the next midnight, so a span of consecutive
            // days cannot claim the same entry twice at its boundary.
            end: to.and_hms_opt(23, 59, 59).expect("23:59:59 always exists"),
            page: 1,
            size: PAGE_SIZE,
        }
    }

    fn query_string(&self) -> String {
        let stamp = |at: NaiveDateTime| at.format("%Y-%m-%dT%H:%M:%S").to_string();
        format!(
            "timesheets?begin={}&end={}&page={}&size={}",
            stamp(self.begin),
            stamp(self.end),
            self.page,
            self.size
        )
    }
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
    token: String,
}

impl Client {
    pub fn new(base: &str, token: String) -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(REQUEST_TIMEOUT))
                .build()
                .into(),
            // Trailing slashes would double up in every request path.
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    /// Whatever clock is running right now, newest first.
    pub fn active(&self) -> Result<Vec<Entry>> {
        let body = self.get("timesheets/active")?;
        serde_json::from_str(&body).context("cannot read the active timesheet")
    }

    /// One page of the collection endpoint.
    pub fn timesheets(&self, query: &Query) -> Result<Vec<Entry>> {
        let body = self.get(&query.query_string())?;
        serde_json::from_str(&body).context("cannot read the timesheet list")
    }

    /// Every entry in the window, following pages until they run out.
    ///
    /// Two things end the walk, and it accepts either without deciding which
    /// Kimai does: a short page, and an error on a page past the first. The
    /// documented behaviour past the last page is a 404, but a version that
    /// returns an empty array instead would also stop here — and getting this
    /// wrong in the other direction means either a truncated month or an
    /// infinite loop.
    pub fn timesheets_all(&self, query: &Query) -> Result<Vec<Entry>> {
        let mut out = Vec::new();
        for page in 1..=MAX_PAGES {
            let mut q = query.clone();
            q.page = page;
            let entries = match self.timesheets(&q) {
                Ok(entries) => entries,
                Err(e) if page > 1 => {
                    // Past the end. Not an error: the pages we did read are the
                    // answer.
                    let _ = e;
                    break;
                }
                Err(e) => return Err(e),
            };
            let short = entries.len() < q.size as usize;
            out.extend(entries);
            if short {
                break;
            }
        }
        Ok(out)
    }

    fn get(&self, path: &str) -> Result<String> {
        Ok(self
            .agent
            .get(format!("{}/api/{path}", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/json")
            .call()?
            .body_mut()
            .read_to_string()?)
    }
}

/// Parse one of Kimai's timestamps into local time.
///
/// Both spellings are accepted rather than betting on one: RFC 3339 for a
/// version that emits it, and the unseparated-offset form the instance actually
/// sends.
pub fn parse_timestamp(raw: &str) -> Result<DateTime<Local>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .or_else(|_| DateTime::<FixedOffset>::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%z"))
        .with_context(|| format!("unreadable timestamp {raw:?}"))?;
    Ok(parsed.with_timezone(&Local))
}

/// Sum the finished entries in a set, in seconds.
///
/// `end.is_some()` is the load-bearing part: the collection endpoint hands back
/// the running record at duration 0 despite the `end` filter, and a caller that
/// counts it forward locally would otherwise have it in the total twice.
pub fn finished_seconds<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> i64 {
    entries
        .into_iter()
        .filter(|entry| entry.end.is_some())
        .map(|entry| entry.duration.unwrap_or(0))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_timestamp_spellings_parse() {
        // What the instance actually sends.
        assert!(parse_timestamp("2026-08-07T15:55:38-0300").is_ok());
        // And what a stricter version would.
        assert!(parse_timestamp("2026-08-07T15:55:38-03:00").is_ok());
        assert!(parse_timestamp("mañana").is_err());
    }

    #[test]
    fn a_span_ends_a_second_before_midnight() {
        let q = Query::span("2026-08-01".parse().unwrap(), "2026-08-31".parse().unwrap());
        let qs = q.query_string();
        assert!(qs.contains("begin=2026-08-01T00:00:00"), "{qs}");
        assert!(qs.contains("end=2026-08-31T23:59:59"), "{qs}");
    }

    #[test]
    fn the_running_entry_is_not_counted_twice() {
        let entries: Vec<Entry> = serde_json::from_str(
            r#"[
              {"id": 1, "begin": "2026-08-11T09:00:00-0300",
               "end": "2026-08-11T12:00:00-0300", "duration": 10800},
              {"id": 2, "begin": "2026-08-11T13:00:00-0300", "duration": 0}
            ]"#,
        )
        .unwrap();

        assert!(entries[1].is_running());
        assert_eq!(finished_seconds(&entries), 10800);
    }

    #[test]
    fn a_task_label_skips_the_half_that_is_missing() {
        let only_project: Entry = serde_json::from_str(
            r#"{"id": 1, "begin": "2026-08-11T09:00:00-0300",
                "project": {"name": "SmartMachine"}}"#,
        )
        .unwrap();
        assert_eq!(only_project.task(), "SmartMachine");

        let both: Entry = serde_json::from_str(
            r#"{"id": 1, "begin": "2026-08-11T09:00:00-0300",
                "project": {"name": "SmartMachine"}, "activity": {"name": "Firmware"}}"#,
        )
        .unwrap();
        assert_eq!(both.task(), "SmartMachine / Firmware");

        let neither: Entry =
            serde_json::from_str(r#"{"id": 1, "begin": "2026-08-11T09:00:00-0300"}"#).unwrap();
        assert_eq!(neither.task(), "");
    }

    /// The shape the collection endpoint actually sends, keys and order as they
    /// come off the wire. Every other fixture here is written expanded, which is
    /// what let a type that could only parse the expanded form look tested: the
    /// fixtures agreed with the struct, and neither agreed with the server.
    #[test]
    fn the_collection_sends_relations_as_bare_ids() {
        let entries: Vec<Entry> = serde_json::from_str(
            r#"[{"activity":667,"project":41,"user":3,
                 "id":9001,"begin":"2026-08-11T09:00:00-0300",
                 "end":"2026-08-11T12:00:00-0300","duration":10800,
                 "description":null,"tags":[]}]"#,
        )
        .expect("a collection response must parse");

        assert_eq!(finished_seconds(&entries), 10800);
        // No name to be had from an id, so the label is empty rather than wrong.
        assert_eq!(entries[0].task(), "");
    }

    /// Both endpoints feed one `Entry`, so one value has to accept both shapes.
    #[test]
    fn one_entry_type_reads_either_endpoint() {
        let expanded: Entry = serde_json::from_str(
            r#"{"id":1,"begin":"2026-08-11T09:00:00-0300",
                "project":{"id":41,"name":"SmartMachine"},
                "activity":{"id":667,"name":"Firmware"}}"#,
        )
        .unwrap();
        assert_eq!(expanded.task(), "SmartMachine / Firmware");

        // A relation that is present but null is not the same as an id.
        let nulled: Entry = serde_json::from_str(
            r#"{"id":1,"begin":"2026-08-11T09:00:00-0300","project":null,"activity":667}"#,
        )
        .unwrap();
        assert_eq!(nulled.task(), "");
    }
}
