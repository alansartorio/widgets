//! The `kimai` CLI: one JSON object per line on stdout, plus the commands that
//! manage the feriado calendar.
//!
//! Two rules run through all of it, the same two the timers CLI follows.
//!
//! **Never fail silently:** a problem the person should fix goes to stderr with
//! a non-zero exit.
//!
//! **Never fail loudly at the bar's expense:** when Kimai cannot be reached,
//! stdout still gets a complete JSON object, so every `?.field` in the widget
//! keeps working and the pill says what is wrong instead of vanishing. `tail`
//! in particular never exits — eww's `deflisten` owns that process for the life
//! of the bar, and an exit would freeze the variable at its last value with
//! nothing to say why.

mod holidays_cmd;

use std::io::Write;
use std::thread::sleep;
use std::time::Duration;

use chrono::Local;
use kimai_core::{Config, Emitter, Output, Poller, describe};

/// How often the running entry is counted forward locally.
///
/// Not a request: the poll rate is in the config. This is the rate at which a
/// running clock reads differently, and the emitter is what stops an idle one
/// from reaching the bar sixty times a minute unchanged.
const TICK: Duration = Duration::from_secs(1);

/// How long `tail` waits before retrying after a failed poll.
///
/// One tick would hammer a server that is merely slow, and a full poll interval
/// would leave the pill saying "unreachable" for a minute after the VPN comes
/// back.
const RETRY: Duration = Duration::from_secs(10);

const USAGE: &str = "\
usage: kimai [command]

  (none) | status           print one reading as JSON, then exit
  tail                      stream JSON, one line per change, until killed
  today | week | month      a human summary
  initial                   the JSON the bar's deflisten should start from

  holidays sync   [--year N] [--force]   fetch the feriados for a year
  holidays list   [--year N] [--month M] [--json]
  holidays set    <YYYY-MM-DD> --kind KIND [--name TEXT]
  holidays unset  <YYYY-MM-DD>
  holidays path                          where the override file lives

  config path | show

  KIND   holiday | non-working | bridge | work

Configuration lives in ~/.config/kimai/config.toml; KIMAI_URL and
KIMAI_TOKEN_FILE override it for a one-off run.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = || args[1..].to_vec();

    match args.first().map(String::as_str) {
        None | Some("status") | Some("once") => once(),
        Some("tail") => tail(),
        // The bar's `deflisten :initial` has to be a literal in the yuck, and a
        // literal is a copy that can drift. This prints the one it should be,
        // so keeping the two in step is a diff rather than an audit.
        Some("initial") => emit_once(Output::failed("starting")),
        Some("today") => summary(Span::Today),
        Some("week") => summary(Span::Week),
        Some("month") => summary(Span::Month),
        Some("holidays") | Some("feriados") => holidays_cmd::run(&rest()),
        Some("config") => config_cmd(&rest()),
        Some("help") | Some("--help") | Some("-h") => print!("{USAGE}"),
        Some(other) => {
            eprintln!("kimai: unknown command {other:?}");
            eprint!("{USAGE}");
            // Still a valid object on stdout: a widget that got here through a
            // typo in the bar config should say something, not break.
            emit_once(Output::failed(&format!("unknown command {other:?}")));
            std::process::exit(2);
        }
    }
}

/// Load the config, or explain why there is none.
///
/// A missing config is not the same as a broken one. The first is a machine
/// that simply has no Kimai — the widget should not be there, and this prints
/// `configured: false` and exits cleanly. The second is a typo somebody has to
/// see.
fn load() -> Option<Config> {
    match Config::load() {
        Ok(Some(cfg)) => Some(cfg),
        Ok(None) => {
            eprintln!(
                "kimai: no config found (looked in {})",
                Config::search_paths()
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            emit_once(Output::unconfigured());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("kimai: {e:#}");
            emit_once(Output::failed(&format!("{e:#}")));
            std::process::exit(2);
        }
    }
}

fn emit_once(output: Output) {
    let line = serde_json::to_string(&output).expect("output is always serialisable");
    let _ = writeln!(std::io::stdout(), "{line}");
}

fn once() {
    let cfg = load().expect("load() exits rather than returning None");
    match Poller::new(cfg).and_then(|mut p| p.tick(Local::now())) {
        Ok(output) => emit_once(output),
        Err(e) => {
            eprintln!("kimai: {}", describe(&e));
            emit_once(Output::failed(&describe(&e)));
            std::process::exit(1);
        }
    }
}

/// The `deflisten` contract: a line per change, forever.
fn tail() -> ! {
    let cfg = load().expect("load() exits rather than returning None");
    let mut emitter = Emitter::new().mirroring(kimai_core::last_line_path());

    // Building the poller reads the token, which may not be written yet on a
    // machine that just booted. Say so on the bar and keep looking, rather than
    // exiting and leaving the variable frozen.
    let mut poller = loop {
        match Poller::new(cfg.clone()) {
            Ok(p) => break p,
            Err(e) => {
                emitter.failure(&describe(&e));
                sleep(RETRY);
            }
        }
    };

    // Refreshing the feriado cache is a network call, and this loop must not
    // block on it: a slow API would stall the pill's clock. So it happens on a
    // thread of its own, and the poller picks the result up off disk on its
    // next cycle.
    spawn_holiday_refresh(cfg.holidays_max_age());

    loop {
        match poller.tick(Local::now()) {
            Ok(output) => {
                emitter.send(output);
                sleep(TICK);
            }
            Err(e) => {
                // A failed request prints failed=true and the loop carries on,
                // so the widget recovers by itself once the VPN or the server
                // comes back.
                emitter.failure(&describe(&e));
                sleep(RETRY);
            }
        }
    }
}

/// Refresh the feriado cache in the background, once at start and once a day.
fn spawn_holiday_refresh(max_age: chrono::Duration) {
    std::thread::spawn(move || {
        loop {
            let mut cal = holidays::Calendar::open(holidays::WorkWeek::default());
            if let holidays::Refresh::Failed(e) = cal.refresh_if_stale(max_age) {
                // stderr, not stdout: stdout is the bar's, and a feriado
                // refresh that failed is a footnote rather than a reading. The
                // widget already shows `holidays_degraded` if it matters.
                eprintln!("kimai: feriados: {e}");
            }
            sleep(Duration::from_secs(6 * 3600));
        }
    });
}

enum Span {
    Today,
    Week,
    Month,
}

fn summary(span: Span) {
    let cfg = load().expect("load() exits rather than returning None");
    let output = match Poller::new(cfg).and_then(|mut p| p.tick(Local::now())) {
        Ok(output) => output,
        Err(e) => {
            eprintln!("kimai: {}", describe(&e));
            std::process::exit(1);
        }
    };

    let (label, booked, expected, delta) = match span {
        Span::Today => (
            "hoy",
            &output.today_time,
            &output.expected_today,
            &output.today_delta,
        ),
        Span::Week => (
            "semana",
            &output.week_time,
            &output.expected_week,
            &output.week_delta,
        ),
        Span::Month => (
            "mes",
            &output.month_time,
            &output.expected_month,
            &output.month_delta,
        ),
    };

    println!("{label}: {booked} / {expected}  ({delta})");
    if output.running {
        println!("corriendo: {} desde {}", output.task, output.since);
    }
    if output.holiday {
        println!("feriado: {}", output.holiday_name);
    }
    if output.holidays_degraded {
        println!("sin datos de feriados — sólo fines de semana");
    }
}

fn config_cmd(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("path") | None => {
            for path in Config::search_paths() {
                let mark = if path.exists() { "*" } else { " " };
                println!("{mark} {}", path.display());
            }
        }
        Some("show") => {
            let cfg = load().expect("load() exits rather than returning None");
            println!("base_url   {}", cfg.base_url);
            println!("token_file {}", cfg.token_file.display());
            println!("jornada    {} h", cfg.daily_target_hours);
            println!(
                "semana     {} (desde {})",
                cfg.workdays.join(" "),
                cfg.week_start
            );
            println!(
                "puentes    {}",
                if cfg.bridges_are_working {
                    "se trabajan"
                } else {
                    "no se trabajan"
                }
            );
            println!(
                "poll       {} s (span {} s)",
                cfg.poll_seconds, cfg.span_poll_seconds
            );
        }
        Some(other) => {
            eprintln!("kimai config: unknown subcommand {other:?}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use kimai_core::Output;

    /// Every field the widget reads must survive a failure, or the pill renders
    /// a row of empty strings that looks like a working zero.
    #[test]
    fn the_failure_shape_is_complete() {
        let json = serde_json::to_value(Output::failed("sin VPN")).unwrap();
        let obj = json.as_object().unwrap();

        for field in [
            "configured",
            "ok",
            "failed",
            "state",
            "running",
            "task",
            "description",
            "since",
            "running_time",
            "today_time",
            "week_time",
            "month_time",
            "expected_today",
            "expected_week",
            "expected_month",
            "today_delta",
            "week_delta",
            "month_delta",
            "day_percent",
            "week_percent",
            "month_percent",
            "holiday",
            "holiday_name",
            "day_kind",
            "holidays_degraded",
            "label",
            "tooltip",
            "error",
        ] {
            assert!(obj.contains_key(field), "missing {field}");
        }

        assert_eq!(obj["failed"], serde_json::json!(true));
        assert_eq!(obj["label"], serde_json::json!("kimai unreachable"));
        assert_eq!(obj["tooltip"], serde_json::json!("kimai: sin VPN"));
    }

    /// `Output` deserializes with no `#[serde(default)]` anywhere, so a literal
    /// that has lost a field fails to parse — which makes this a real check on
    /// the copy in the bar's yuck rather than a restatement of the struct.
    ///
    /// The string below is that copy, verbatim. When it stops matching, the
    /// yuck's `:initial` needs the output of `kimai initial` pasted into it.
    #[test]
    fn the_bars_initial_literal_is_still_the_shape_the_cli_prints() {
        const WIDGET_INITIAL: &str = r#"{"configured":true,"ok":false,"failed":true,"state":"failed","running":false,"task":"","description":"","since":"","running_time":"0s","today_time":"0s","week_time":"0s","month_time":"0s","today_seconds":0,"week_seconds":0,"month_seconds":0,"expected_today":"0s","expected_week":"0s","expected_month":"0s","today_delta":"+0s","week_delta":"+0s","month_delta":"+0s","today_delta_seconds":0,"week_delta_seconds":0,"month_delta_seconds":0,"day_percent":0,"week_percent":0,"month_percent":0,"holiday":false,"holiday_name":"","day_kind":"working","holidays_degraded":true,"week_start":"","week_end":"","month_label":"","label":"kimai unreachable","tooltip":"kimai: starting","error":"starting"}"#;

        let parsed: Output = serde_json::from_str(WIDGET_INITIAL)
            .expect("the bar's initial value no longer parses as an Output");
        assert_eq!(parsed, Output::failed("starting"));
    }

    #[test]
    fn an_unconfigured_machine_says_so_rather_than_failing() {
        let out = Output::unconfigured();
        assert!(!out.configured);
        // Still failed, so a widget that only checks `failed` does the right
        // thing; `configured` is what hides the pill entirely.
        assert!(out.failed);
    }
}
