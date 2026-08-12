//! `kimai holidays …` — the feriado calendar's front end.
//!
//! It lives on this binary because the `holidays` crate ships no binary of its
//! own, and a fifth one on PATH to manage a file two other tools read would be
//! a worse trade than a subcommand here. `calendar-gui` reads the same files
//! and needs no commands: whatever you set here, it shows.

use anyhow::{Context, Result, anyhow};
use chrono::{Datelike, Local, NaiveDate};
use holidays::{Calendar, DayKind, WorkWeek};

const USAGE: &str = "\
usage: kimai holidays <command>

  sync  [--year N] [--force]   fetch the feriados from api.argentinadatos.com
  list  [--year N] [--month M] [--json]
  set   <YYYY-MM-DD> --kind KIND [--name TEXT]
  unset <YYYY-MM-DD>
  path                         print the override file's location

  KIND   holiday       feriado — no se trabaja
         non-working   día no laborable (Jueves Santo y compañía)
         bridge        puente turístico
         work          se trabaja, aunque el feriado diga otra cosa
";

pub fn run(args: &[String]) {
    if let Err(e) = dispatch(args) {
        eprintln!("kimai holidays: {e:#}");
        std::process::exit(1);
    }
}

fn dispatch(args: &[String]) -> Result<()> {
    let rest = || args[1..].to_vec();
    match args.first().map(String::as_str) {
        Some("sync") | Some("fetch") => sync(&rest()),
        Some("list") | Some("ls") | None => list(&rest()),
        Some("set") => set(&rest()),
        Some("unset") | Some("rm") => unset(&rest()),
        Some("path") => {
            println!("{}", locations().overrides.display());
            println!("{}", locations().state_dir.display());
            Ok(())
        }
        Some("help") | Some("--help") | Some("-h") => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => {
            eprint!("{USAGE}");
            Err(anyhow!("unknown subcommand {other:?}"))
        }
    }
}

fn locations() -> holidays::Locations {
    holidays::Locations::from_env()
}

/// The work week these commands read the calendar with.
///
/// From the kimai config when there is one, so `list` agrees with what the pill
/// counts, and the default otherwise — these commands have to work on a machine
/// where Kimai is not set up but the calendar is.
fn work_week() -> WorkWeek {
    kimai_core::Config::load()
        .ok()
        .flatten()
        .map(|cfg| cfg.work_week())
        .unwrap_or_default()
}

fn open() -> Calendar {
    Calendar::open(work_week())
}

fn sync(args: &[String]) -> Result<()> {
    let year = flag_value(args, "--year")
        .map(|v| v.parse::<i32>().context("--year wants a year"))
        .transpose()?
        .unwrap_or_else(|| Local::now().year());
    let force = args.iter().any(|a| a == "--force");

    let mut cal = open();
    cal.ensure_year(year);

    if force {
        cal.refresh_year(year)?;
        println!("feriados {year}: actualizados");
        return Ok(());
    }

    match cal.refresh_if_stale(
        kimai_core::Config::load()
            .ok()
            .flatten()
            .map(|c| c.holidays_max_age())
            .unwrap_or_else(|| chrono::Duration::days(7)),
    ) {
        holidays::Refresh::Fresh => println!("feriados: ya estaban al día"),
        holidays::Refresh::Updated => println!("feriados: actualizados"),
        holidays::Refresh::Failed(e) => return Err(anyhow!(e)),
    }
    Ok(())
}

fn list(args: &[String]) -> Result<()> {
    let now = Local::now().date_naive();
    let year = flag_value(args, "--year")
        .map(|v| v.parse::<i32>().context("--year wants a year"))
        .transpose()?
        .unwrap_or_else(|| now.year());
    let month = flag_value(args, "--month")
        .map(|v| parse_month(&v))
        .transpose()?;
    let as_json = args.iter().any(|a| a == "--json");

    let mut cal = open();
    cal.ensure_year(year);

    let months: Vec<u32> = match month {
        Some(m) => vec![m],
        None => (1..=12).collect(),
    };

    // Not just `view.holidays`, which is the *non-working* days the calendar
    // window colours. A `work` override on Viernes Santo is exactly the sort of
    // thing you run this command to check, and it would be invisible in a list
    // of days off.
    let mut rows = Vec::new();
    for m in months {
        for cell in cal.month_view(year, m).cells {
            if !cell.in_month {
                continue;
            }
            if cell.day.name.is_some() || cell.day.overridden {
                rows.push((cell.day, cell.is_working));
            }
        }
    }

    if as_json {
        println!(
            "{}",
            serde_json::to_string(&rows.iter().map(|(d, _)| d).collect::<Vec<_>>())?
        );
        return Ok(());
    }

    if rows.is_empty() {
        println!("(sin feriados — ¿corriste `kimai holidays sync`?)");
        return Ok(());
    }

    for (day, is_working) in rows {
        println!(
            "{}  {:<12} {}{}{}",
            day.date,
            day.kind.as_str(),
            day.name.as_deref().unwrap_or(""),
            if day.overridden { "  [a mano]" } else { "" },
            if is_working { "  [se trabaja]" } else { "" }
        );
    }

    if cal.coverage().degraded {
        println!("\nsin datos para {year} — sólo fines de semana");
    }
    Ok(())
}

fn set(args: &[String]) -> Result<()> {
    let date = positional(args).ok_or_else(|| anyhow!("which date? (YYYY-MM-DD)"))?;
    let date: NaiveDate = date
        .parse()
        .with_context(|| format!("{date:?} is not a YYYY-MM-DD date"))?;

    let kind_raw = flag_value(args, "--kind").ok_or_else(|| anyhow!("--kind is required"))?;
    let kind = DayKind::parse(&kind_raw)
        .ok_or_else(|| anyhow!("unknown kind {kind_raw:?} — see `kimai holidays help`"))?;
    let name = flag_value(args, "--name");

    let mut cal = open();
    cal.set_override(date, kind, name.as_deref())?;

    println!(
        "{date}: {} — {}",
        kind.as_str(),
        if cal.is_working_day(date) {
            "se trabaja"
        } else {
            "no se trabaja"
        }
    );
    Ok(())
}

fn unset(args: &[String]) -> Result<()> {
    let date = positional(args).ok_or_else(|| anyhow!("which date? (YYYY-MM-DD)"))?;
    let date: NaiveDate = date
        .parse()
        .with_context(|| format!("{date:?} is not a YYYY-MM-DD date"))?;

    let mut cal = open();
    cal.unset_override(date)?;

    let day = cal.day(date);
    println!(
        "{date}: {} — {}",
        day.kind.as_str(),
        if cal.is_working_day(date) {
            "se trabaja"
        } else {
            "no se trabaja"
        }
    );
    Ok(())
}

/// `--flag value`, or `--flag=value`.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let prefix = format!("{flag}=");
    for (i, arg) in args.iter().enumerate() {
        if let Some(v) = arg.strip_prefix(&prefix) {
            return Some(v.to_string());
        }
        if arg == flag {
            return args.get(i + 1).cloned();
        }
    }
    None
}

/// The first argument that is neither a flag nor a flag's value.
fn positional(args: &[String]) -> Option<String> {
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg.starts_with("--") {
            skip_next = !arg.contains('=');
            continue;
        }
        return Some(arg.clone());
    }
    None
}

/// `8`, `08` or `2026-08` — the last spelling because that is what
/// `month_label` in the JSON says, and copying it out should work.
fn parse_month(raw: &str) -> Result<u32> {
    let tail = raw.rsplit('-').next().unwrap_or(raw);
    let month: u32 = tail
        .parse()
        .with_context(|| format!("{raw:?} is not a month"))?;
    anyhow::ensure!((1..=12).contains(&month), "{raw:?} is not a month");
    Ok(month)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_take_both_spellings() {
        assert_eq!(
            flag_value(&args(&["--year", "2026"]), "--year").as_deref(),
            Some("2026")
        );
        assert_eq!(
            flag_value(&args(&["--year=2026"]), "--year").as_deref(),
            Some("2026")
        );
        assert_eq!(flag_value(&args(&["--force"]), "--year"), None);
    }

    #[test]
    fn a_date_is_found_past_its_flags() {
        assert_eq!(
            positional(&args(&["--kind", "work", "2026-04-03"])).as_deref(),
            Some("2026-04-03")
        );
        assert_eq!(
            positional(&args(&["2026-04-03", "--kind", "work"])).as_deref(),
            Some("2026-04-03")
        );
        // `--kind=work` consumes no following argument.
        assert_eq!(
            positional(&args(&["--kind=work", "2026-04-03"])).as_deref(),
            Some("2026-04-03")
        );
    }

    #[test]
    fn months_take_the_label_spelling_too() {
        assert_eq!(parse_month("8").unwrap(), 8);
        assert_eq!(parse_month("08").unwrap(), 8);
        assert_eq!(parse_month("2026-08").unwrap(), 8);
        assert!(parse_month("13").is_err());
        assert!(parse_month("agosto").is_err());
    }
}
