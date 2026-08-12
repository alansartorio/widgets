//! The arithmetic, against the real 2026 calendar.
//!
//! Every fixture below is the payload
//! `https://api.argentinadatos.com/v1/feriados/2026` actually returned, so a
//! change in the API's shape breaks these rather than the bar.

use chrono::{Datelike, Duration, Local, NaiveDate};

use crate::cache::{self, YearFile};
use crate::calendar::{Calendar, Locations};
use crate::model::{DayKind, WorkWeek};
use crate::source::ApiDay;

/// The nineteen entries for 2026, verbatim.
fn feriados_2026() -> Vec<ApiDay> {
    let raw = [
        ("2026-01-01", "inamovible", "Año nuevo"),
        ("2026-02-16", "inamovible", "Carnaval"),
        ("2026-02-17", "inamovible", "Carnaval"),
        ("2026-03-23", "puente", "Puente turístico no laborable"),
        ("2026-03-24", "inamovible", "Día Nacional de la Memoria"),
        ("2026-04-02", "inamovible", "Día del Veterano"),
        ("2026-04-03", "inamovible", "Viernes Santo"),
        ("2026-05-01", "inamovible", "Día del Trabajador"),
        ("2026-05-25", "inamovible", "Día de la Revolución de Mayo"),
        ("2026-06-15", "trasladable", "Güemes (17/6)"),
        ("2026-06-20", "inamovible", "Belgrano"),
        ("2026-07-09", "inamovible", "Día de la Independencia"),
        ("2026-07-10", "puente", "Puente turístico no laborable"),
        ("2026-08-17", "trasladable", "San Martín"),
        ("2026-10-12", "trasladable", "Diversidad Cultural"),
        ("2026-11-23", "trasladable", "Soberanía Nacional (20/11)"),
        ("2026-12-07", "puente", "Puente turístico no laborable"),
        ("2026-12-08", "inamovible", "Inmaculada Concepción"),
        ("2026-12-25", "inamovible", "Navidad"),
    ];
    raw.iter()
        .map(|(fecha, tipo, nombre)| ApiDay {
            fecha: fecha.to_string(),
            tipo: tipo.to_string(),
            nombre: nombre.to_string(),
        })
        .collect()
}

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

/// A scratch directory of its own per test, without a dev-dependency for it.
fn scratch(name: &str) -> Locations {
    let dir = std::env::temp_dir().join(format!("holidays-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Locations {
        state_dir: dir.join("state"),
        overrides: dir.join("overrides.toml"),
    }
}

/// A calendar with 2026 cached and nothing overridden.
fn cal_2026(name: &str) -> Calendar {
    let locations = scratch(name);
    cache::store(
        &locations.state_dir,
        &YearFile {
            year: 2026,
            fetched_at: Local::now(),
            source: "test".into(),
            days: feriados_2026(),
        },
    )
    .unwrap();

    let mut cal = Calendar::open_at(locations, WorkWeek::default());
    cal.ensure_year(2026);
    cal
}

#[test]
fn a_feriado_is_not_a_working_day() {
    let cal = cal_2026("feriado");
    // 2026-05-01 is a Friday, and the Día del Trabajador.
    assert_eq!(d("2026-05-01").weekday().to_string(), "Fri");
    assert!(!cal.is_working_day(d("2026-05-01")));
    assert_eq!(cal.day(d("2026-05-01")).kind, DayKind::Holiday);
}

#[test]
fn a_puente_is_worked_unless_told_otherwise_is_backwards() {
    // The default is the cautious one: a puente is *not* worked, because that
    // is what "no laborable" means until an employer says otherwise.
    let cal = cal_2026("puente-default");
    assert_eq!(cal.day(d("2026-07-10")).kind, DayKind::Bridge);
    assert!(!cal.is_working_day(d("2026-07-10")));
}

#[test]
fn bridges_are_working_flips_the_puentes_and_nothing_else() {
    let locations = scratch("puente-on");
    cache::store(
        &locations.state_dir,
        &YearFile {
            year: 2026,
            fetched_at: Local::now(),
            source: "test".into(),
            days: feriados_2026(),
        },
    )
    .unwrap();
    let week = WorkWeek {
        bridges_are_working: true,
        ..WorkWeek::default()
    };
    let mut cal = Calendar::open_at(locations, week);
    cal.ensure_year(2026);

    // 2026-07-10 is a Friday puente: now worked.
    assert!(cal.is_working_day(d("2026-07-10")));
    // 2026-07-09 is the Día de la Independencia: still not.
    assert!(!cal.is_working_day(d("2026-07-09")));
}

#[test]
fn a_work_override_beats_a_feriado() {
    let mut cal = cal_2026("override-work");
    assert!(!cal.is_working_day(d("2026-04-03"))); // Viernes Santo

    cal.set_override(d("2026-04-03"), DayKind::ForcedWork, None)
        .unwrap();

    assert!(cal.is_working_day(d("2026-04-03")));
    let day = cal.day(d("2026-04-03"));
    assert_eq!(day.kind, DayKind::ForcedWork);
    assert!(day.overridden);
    // The API's name survives the override, so the calendar can still say what
    // the day is while counting it.
    assert_eq!(day.name.as_deref(), Some("Viernes Santo"));
}

#[test]
fn an_override_adds_a_day_the_api_never_heard_of() {
    let mut cal = cal_2026("override-add");
    // Jueves Santo 2026 — a día no laborable, which the API does not list at
    // all, so a plain Thursday until someone says otherwise.
    assert!(cal.is_working_day(d("2026-04-01")));
    assert_eq!(cal.day(d("2026-04-01")).kind, DayKind::Working);

    cal.set_override(d("2026-04-01"), DayKind::NonWorking, Some("Jueves Santo"))
        .unwrap();

    assert!(!cal.is_working_day(d("2026-04-01")));
    assert_eq!(
        cal.day(d("2026-04-01")).name.as_deref(),
        Some("Jueves Santo")
    );
}

#[test]
fn unsetting_an_override_puts_the_day_back() {
    let mut cal = cal_2026("override-unset");
    cal.set_override(d("2026-04-03"), DayKind::ForcedWork, None)
        .unwrap();
    assert!(cal.is_working_day(d("2026-04-03")));

    cal.unset_override(d("2026-04-03")).unwrap();
    assert!(!cal.is_working_day(d("2026-04-03")));
    assert!(!cal.day(d("2026-04-03")).overridden);
}

#[test]
fn a_week_with_a_feriado_expects_four_days() {
    let cal = cal_2026("week");
    // The week of Mon 2026-05-25, the Revolución de Mayo — a Monday.
    let week = cal.week_of(d("2026-05-27"));
    assert_eq!(*week.start(), d("2026-05-25"));
    assert_eq!(*week.end(), d("2026-05-31"));
    assert_eq!(cal.working_days(week.clone()), 4);
    assert_eq!(cal.expected(week), Duration::hours(32));
}

#[test]
fn expected_through_stops_at_today_inclusive() {
    let cal = cal_2026("through");
    let week = cal.week_of(d("2026-05-27")); // Mon 25 (feriado) .. Sun 31

    // Wednesday: Tue and Wed count, Monday is the feriado.
    assert_eq!(
        cal.expected_through(week.clone(), d("2026-05-27")),
        Duration::hours(16)
    );
    // Monday itself, the feriado: nothing is expected yet.
    assert_eq!(
        cal.expected_through(week.clone(), d("2026-05-25")),
        Duration::zero()
    );
    // A date before the range starts.
    assert_eq!(
        cal.expected_through(week, d("2026-05-20")),
        Duration::zero()
    );
}

#[test]
fn a_month_adds_up() {
    let cal = cal_2026("month");
    let may = cal.month_of(d("2026-05-15"));
    assert_eq!(*may.start(), d("2026-05-01"));
    assert_eq!(*may.end(), d("2026-05-31"));

    // May 2026 has 21 weekdays; the 1st (Fri) and the 25th (Mon) are feriados.
    assert_eq!(cal.working_days(may.clone()), 19);
    assert_eq!(cal.expected(may), Duration::hours(152));
}

#[test]
fn december_rolls_into_january() {
    let cal = cal_2026("december");
    let dec = cal.month_of(d("2026-12-15"));
    assert_eq!(*dec.end(), d("2026-12-31"));
}

#[test]
fn a_year_with_no_cache_is_degraded_and_still_answers() {
    let cal = Calendar::open_at(scratch("degraded"), WorkWeek::default());
    // No panic, no error: weekdays are working days and weekends are not.
    assert!(cal.is_working_day(d("2026-05-01"))); // wrong, and knowably so
    assert!(!cal.is_working_day(d("2026-05-02"))); // a Saturday
    assert!(cal.coverage().degraded);
}

#[test]
fn the_month_grid_is_always_six_weeks() {
    let cal = cal_2026("grid");
    for month in 1..=12 {
        let view = cal.month_view(2026, month);
        assert_eq!(view.cells.len(), crate::month::CELLS, "month {month}");
        // The first cell is always the week start.
        assert_eq!(
            view.cells[0].day.date.weekday(),
            cal.week().week_start,
            "month {month}"
        );
    }
}

#[test]
fn the_grid_names_its_holidays_and_classes_them() {
    let cal = cal_2026("classes");
    let view = cal.month_view(2026, 5);

    assert_eq!(view.title, "mayo 2026");
    assert_eq!(view.weekday_labels, ["L", "M", "M", "J", "V", "S", "D"]);
    assert!(!view.degraded);

    let names: Vec<_> = view
        .holidays
        .iter()
        .filter_map(|d| d.name.clone())
        .collect();
    assert!(names.iter().any(|n| n.contains("Trabajador")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("Revolución")), "{names:?}");

    let first_may = view
        .cells
        .iter()
        .find(|c| c.day.date == d("2026-05-01"))
        .unwrap();
    assert!(first_may.classes().contains(&"holiday"));
    assert!(!first_may.is_working);
}
