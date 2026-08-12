//! Parsing the two ways a timer's finish is expressed: a timespan from now, or
//! a target wall-clock time.
//!
//! Both the CLI and the GUI parse here rather than each rolling its own, so
//! `25m` typed into the add window and `25m` typed at a shell mean the same
//! thing, and an error message about a typo reads the same in both places.

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

/// Seconds from now, from a timespan.
///
/// Accepts unit-suffixed spans (`90s`, `25m`, `2h`, `1h30m`, `1h30`), colon
/// notation (`5:00` = 5 min, `1:05:00` = 1 h 5 min), and a bare number, which
/// means **minutes** — `timers add 25 tea` is the common case and typing `25m`
/// every time is friction for no gain.
pub fn parse_in(spec: &str) -> Result<i64> {
    let spec = spec.trim();
    if spec.is_empty() {
        bail!("no duration given");
    }
    // A leading '+' is how `parse_auto` lets you force a colon form to mean a
    // duration rather than a time of day.
    let spec = spec.strip_prefix('+').unwrap_or(spec);

    let secs = if spec.contains(':') {
        parse_colon_span(spec)?
    } else if spec.chars().all(|c| c.is_ascii_digit()) {
        spec.parse::<i64>()
            .with_context(|| format!("{spec} is too large to be a number of minutes"))?
            .checked_mul(60)
            .ok_or_else(|| anyhow!("{spec} minutes is too far away"))?
    } else {
        parse_unit_span(spec)?
    };

    if secs <= 0 {
        bail!("a duration has to be more than zero");
    }
    Ok(secs)
}

/// An absolute unix-second deadline, from a target wall-clock time.
///
/// Accepts `15:30`, `15:30:45`, the colonless `1530` and `153045`, and a full
/// `2026-08-10 15:30` (or `2026-08-10T15:30`).
///
/// A bare time always resolves to the next time the clock will read it. At
/// 16:00, `1530` means tomorrow at 15:30 — the only other reading is a deadline
/// half an hour in the past, which is not a timer. Give an explicit date to say
/// something this rule would not.
pub fn parse_at(spec: &str) -> Result<i64> {
    let spec = spec.trim();
    if spec.is_empty() {
        bail!("no time given");
    }

    // Split a date off the front if there is one, then parse the rest as a time.
    let (date, time_part) = match spec.split_once(['T', ' ']) {
        Some((date, rest)) if date.contains('-') => {
            let date = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
                .with_context(|| format!("{date} is not a YYYY-MM-DD date"))?;
            (Some(date), rest.trim())
        }
        _ => (None, spec),
    };

    let time = parse_clock(time_part)?;

    match date {
        Some(date) => to_unix(date.and_time(time)),
        None => {
            let today = Local::now().date_naive();
            let candidate = to_unix(today.and_time(time))?;
            if candidate > crate::now() {
                Ok(candidate)
            } else {
                // Rolling forward a whole day rather than adding 86400 seconds
                // keeps the wall-clock time right across a DST boundary.
                let tomorrow = today
                    .succ_opt()
                    .ok_or_else(|| anyhow!("{spec} is past the end of the calendar"))?;
                to_unix(tomorrow.and_time(time))
            }
        }
    }
}

/// An absolute deadline from either form, guessing which was meant.
///
/// A spec with a `:` and no unit letters and no leading `+` is a time of day;
/// everything else is a duration. So `timers add 15:30 standup` and
/// `timers add 25m pasta` both do the obvious thing, and `+5:00` forces the
/// ambiguous colon form to mean five minutes. `--in` / `--at` exist for when
/// you would rather not rely on the guess.
pub fn parse_auto(spec: &str) -> Result<i64> {
    let trimmed = spec.trim();
    let looks_like_clock = trimmed.contains(':')
        && !trimmed.starts_with('+')
        && !trimmed.chars().any(|c| c.is_ascii_alphabetic());

    if looks_like_clock {
        parse_at(trimmed)
    } else {
        Ok(crate::now() + parse_in(trimmed)?)
    }
}

/// `1h30m`, `90s`, `2h`, `1h30` (a trailing bare number takes the next unit
/// down, which is how people write it).
fn parse_unit_span(spec: &str) -> Result<i64> {
    let mut total: i64 = 0;
    let mut digits = String::new();
    let mut last_unit: Option<char> = None;
    let mut saw_any = false;

    for c in spec.chars() {
        match c {
            '0'..='9' => digits.push(c),
            'h' | 'm' | 's' | 'H' | 'M' | 'S' => {
                let unit = c.to_ascii_lowercase();
                let n: i64 = digits
                    .parse()
                    .with_context(|| format!("{spec}: no number before '{c}'"))?;
                digits.clear();
                total = total
                    .checked_add(n.checked_mul(unit_secs(unit)).unwrap_or(i64::MAX))
                    .ok_or_else(|| anyhow!("{spec} is too far away"))?;
                last_unit = Some(unit);
                saw_any = true;
            }
            ' ' => {}
            _ => bail!("{spec}: '{c}' is not part of a duration (try 25m, 1h30m, 90s)"),
        }
    }

    // A trailing bare number after a unit means the next unit down: `1h30` is
    // 90 minutes, not 1 hour and 30 of nothing.
    if !digits.is_empty() {
        let n: i64 = digits.parse().context("trailing number is too large")?;
        let unit = match last_unit {
            Some('h') => 'm',
            Some('m') => 's',
            // `30s5` has nowhere to go, and a bare `30` never reaches here —
            // parse_in handles all-digits before calling us. `last_unit` only
            // ever holds one of the three, so the catch-all is the same case.
            _ => bail!("{spec} is not a duration (try 25m, 1h30m, 90s)"),
        };
        total += n * unit_secs(unit);
        saw_any = true;
    }

    if !saw_any {
        bail!("{spec} is not a duration (try 25m, 1h30m, 90s)");
    }
    Ok(total)
}

/// `5:00` as a span of 5 minutes, `1:05:00` as 1 h 5 min.
fn parse_colon_span(spec: &str) -> Result<i64> {
    let parts: Vec<&str> = spec.split(':').collect();
    let nums: Result<Vec<i64>> = parts
        .iter()
        .map(|p| {
            p.trim()
                .parse::<i64>()
                .with_context(|| format!("{spec}: '{p}' is not a number"))
        })
        .collect();
    let nums = nums?;
    match nums.as_slice() {
        [m, s] => Ok(m * 60 + s),
        [h, m, s] => Ok(h * 3600 + m * 60 + s),
        _ => bail!("{spec} is not a duration (try 5:00 or 1:05:00)"),
    }
}

/// `15:30`, `15:30:45`, or the same without the colons: `1530`, `153045`.
///
/// The colonless forms are here because typing a colon is the slowest character
/// on the row and `1530` is unambiguous once you already know a time of day is
/// what is wanted. Note this is only reachable from [`parse_at`] — a bare number
/// on the duration side still means minutes, and nothing changes there.
fn parse_clock(spec: &str) -> Result<NaiveTime> {
    for fmt in ["%H:%M:%S", "%H:%M"] {
        if let Ok(t) = NaiveTime::parse_from_str(spec, fmt) {
            return Ok(t);
        }
    }

    if !spec.is_empty() && spec.chars().all(|c| c.is_ascii_digit()) {
        // Read from the right: the last two digits are minutes, anything before
        // them is the hour. That makes `930` 9:30 rather than 93:0, and keeps
        // `1530` and `0930` reading the same way.
        let (h, m, s) = match spec.len() {
            1 | 2 => (spec, "0", "0"),
            3 => (&spec[..1], &spec[1..3], "0"),
            4 => (&spec[..2], &spec[2..4], "0"),
            6 => (&spec[..2], &spec[2..4], &spec[4..6]),
            // 5 digits has no reading anyone means, and 7+ is a typo.
            _ => bail!("{spec} is not a time of day (try 1530, 15:30 or 153045)"),
        };
        let parse = |part: &str| part.parse::<u32>().unwrap_or(u32::MAX);
        return NaiveTime::from_hms_opt(parse(h), parse(m), parse(s)).ok_or_else(|| {
            anyhow!("{spec} is not a real time of day (hours 0-23, minutes and seconds 0-59)")
        });
    }

    bail!("{spec} is not a time of day (try 15:30 or 1530)")
}

fn unit_secs(unit: char) -> i64 {
    match unit {
        'h' => 3600,
        'm' => 60,
        _ => 1,
    }
}

/// A local naive datetime as unix seconds.
///
/// The spring-forward gap is the only interesting case: 02:30 simply does not
/// exist on that day, so there is nothing to return but an error saying so.
fn to_unix(dt: NaiveDateTime) -> Result<i64> {
    use chrono::offset::LocalResult;
    match Local.from_local_datetime(&dt) {
        LocalResult::Single(t) => Ok(t.timestamp()),
        // A DST fold: the time happens twice. Take the first, which is the one
        // that arrives sooner.
        LocalResult::Ambiguous(first, _) => Ok(first.timestamp()),
        LocalResult::None => bail!("{dt} does not exist in this timezone (clocks skip forward)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_in_every_notation() {
        assert_eq!(parse_in("90s").unwrap(), 90);
        assert_eq!(parse_in("25m").unwrap(), 1500);
        assert_eq!(parse_in("2h").unwrap(), 7200);
        assert_eq!(parse_in("1h30m").unwrap(), 5400);
        assert_eq!(
            parse_in("1h30").unwrap(),
            5400,
            "trailing number takes the next unit down"
        );
        assert_eq!(parse_in("5:00").unwrap(), 300);
        assert_eq!(parse_in("1:05:00").unwrap(), 3900);
        assert_eq!(parse_in("25").unwrap(), 1500, "a bare number is minutes");
        assert_eq!(parse_in("+25m").unwrap(), 1500);
    }

    #[test]
    fn junk_and_zero_are_rejected() {
        for bad in ["", "0", "0m", "banana", "25x", "m", "1:2:3:4"] {
            assert!(parse_in(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn times_parse_with_or_without_colons() {
        // Same instant however it is typed.
        for (a, b) in [("1530", "15:30"), ("153045", "15:30:45"), ("0930", "09:30")] {
            assert_eq!(parse_at(a).unwrap(), parse_at(b).unwrap(), "{a} != {b}");
        }
        // Three digits read from the right: 9:30, not 93:0.
        assert_eq!(parse_at("930").unwrap(), parse_at("09:30").unwrap());
        // One or two digits are a bare hour.
        assert_eq!(parse_at("15").unwrap(), parse_at("15:00").unwrap());
        assert_eq!(parse_at("7").unwrap(), parse_at("07:00").unwrap());
        // Works with an explicit date in front of it too.
        assert_eq!(
            parse_at("2026-08-10 1530").unwrap(),
            parse_at("2026-08-10 15:30").unwrap()
        );
    }

    #[test]
    fn compact_times_that_are_not_times_are_rejected() {
        // Not real clock readings, and one length with no sensible reading.
        for bad in ["2530", "1560", "153060", "12345", "1234567"] {
            assert!(parse_at(bad).is_err(), "{bad:?} should not parse as a time");
        }
    }

    #[test]
    fn a_bare_time_resolves_to_the_next_time_the_clock_reads_it() {
        // Pick a target an hour behind the current time, in both notations, and
        // check it lands tomorrow rather than in the past.
        let now = Local::now();
        let past = now - chrono::Duration::hours(1);
        let (with_colon, compact) = (
            past.format("%H:%M").to_string(),
            past.format("%H%M").to_string(),
        );

        let resolved = parse_at(&compact).unwrap();
        assert_eq!(resolved, parse_at(&with_colon).unwrap());
        assert!(
            resolved > crate::now(),
            "an hour ago resolved into the past"
        );
        // Tomorrow, not some later day: within 24h of now.
        assert!(resolved <= crate::now() + 24 * 3600);
    }

    #[test]
    fn a_bare_time_never_lands_in_the_past() {
        // Whatever the clock says while the test runs, both of these are ahead
        // of it — one of them by rolling to tomorrow.
        for spec in ["00:01", "23:59"] {
            assert!(
                parse_at(spec).unwrap() > crate::now(),
                "{spec} resolved into the past"
            );
        }
    }

    #[test]
    fn auto_tells_a_clock_from_a_duration() {
        let now = crate::now();
        // 15:30 is a time of day: at most 24 h out, and not 15 h 30 m of span.
        let clock = parse_auto("15:30").unwrap();
        assert!(clock > now && clock <= now + 24 * 3600);
        // A leading + forces the same text to mean a span.
        assert_eq!(parse_auto("+15:30").unwrap() - now, 15 * 60 + 30);
        // Unit letters are unambiguous.
        assert_eq!(parse_auto("25m").unwrap() - now, 1500);
    }
}
