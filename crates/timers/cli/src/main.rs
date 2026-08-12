//! The `timers` CLI: one JSON object per line, on stdout.
//!
//! Every subcommand prints the same shape — the daemon answers every mutation
//! with the resulting snapshot — so a script never has to follow a write with a
//! read, and the eww bar can `deflisten` on `timers tail` without special-casing
//! anything.
//!
//! Two rules run through all of it. Never fail silently: a problem the person
//! should fix goes to stderr with a non-zero exit. And never fail loudly at the
//! bar's expense: when the daemon cannot be reached, stdout still gets a
//! complete JSON object, so every `?.field` in the widget keeps working and the
//! pill says what is wrong instead of vanishing.

use std::io::Write;
use std::time::Duration;

use timers_core::client;
use timers_core::models::Status;
use timers_core::paths::{Paths, TimersPaths};
use timers_core::proto::{Request, Response};
use timers_core::timespec;

/// What the bar reads when there is no daemon to ask.
///
/// A literal rather than `serde_json::to_string(&Status::failed())` because the
/// whole point of it is to be printable when nothing else is working. It has to
/// stay field-for-field identical to `Status::failed()`; `fallback_matches`
/// below is what enforces that, and it is the reason this file has a test at
/// all.
const FALLBACK: &str = r#"{"ok":false,"failed":true,"count":0,"pending":0,"firing":0,"state":"failed","name":"","label":"timers daemon unreachable","remaining":0,"pct":0,"tooltip":"no answer from the timers daemon · see eww logs","at":0,"timers":[]}"#;

/// How long `tail` waits before trying the daemon again.
///
/// Shorter than a usage poller's would be: a timer that is late is worse than a
/// number that is stale, and reconnecting to a unix socket costs nothing.
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

const USAGE: &str = "\
usage: timers [command]

  (none) | list | ls        print the current timers as JSON
  tail                      stream JSON, one line per update, until killed
  add [--in SPEC | --at TIME | SPEC] [--private] NAME
  rm ID...                  delete timers, fired or not
  ack ID... | ack --all     acknowledge fired timers (deletes them)

  SPEC   25m · 1h30m · 90s · 5:00 · 25 (bare number means minutes)
  TIME   1530 · 15:30 · 153045 · 2026-08-10 15:30
         a bare time means the next time the clock reads it, so at 16:00
         `1530` is tomorrow

  --private   keep the name off the eww bar: the pill, its tooltip and the
              notification all say \"timer\". The popup still shows it.
";

fn main() {
    let paths = Paths::from_env();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = || args[1..].to_vec();

    match args.first().map(String::as_str) {
        None | Some("list") | Some("ls") | Some("status") => once(&paths, Request::List),
        Some("tail") => tail(&paths),
        Some("add") | Some("new") => add(&paths, &rest()),
        Some("rm") | Some("remove") | Some("del") => remove(&paths, &rest()),
        Some("ack") | Some("dismiss") => ack(&paths, &rest()),
        Some("help") | Some("--help") | Some("-h") => {
            print!("{USAGE}");
        }
        Some(other) => {
            eprintln!("timers: unknown command {other:?}");
            eprint!("{USAGE}");
            // Still a valid object on stdout: a widget that got here through a
            // typo in the bar config should say something, not break.
            emit_fallback();
            std::process::exit(2);
        }
    }
}

/// One request, one JSON line.
fn once(paths: &Paths, req: Request) {
    match client::request(paths, &req) {
        Ok(Response::Status(status)) => emit(&status),
        Ok(Response::Error(msg)) => {
            eprintln!("timers: {msg}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("timers: daemon unreachable: {e:#}");
            emit_fallback();
            std::process::exit(1);
        }
    }
}

fn add(paths: &Paths, args: &[String]) {
    let (deadline, name, private) = match parse_add(args) {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!("timers: {e:#}");
            // Nothing on stdout: a mutation that did not happen should not
            // print a status line a script might mistake for success.
            std::process::exit(2);
        }
    };
    once(
        paths,
        Request::Add {
            name,
            deadline,
            private,
        },
    );
}

/// Pull the deadline, the name and the hidden flag out of `add`'s arguments.
///
/// `--in`/`--at` take the next argument; without either, the first argument is
/// the spec and `timespec::parse_auto` decides which kind it is. Everything
/// left over joins into the name, so quoting it is optional.
fn parse_add(args: &[String]) -> anyhow::Result<(i64, String, bool)> {
    let mut deadline: Option<i64> = None;
    let mut private = false;
    let mut words: Vec<&str> = Vec::new();
    let mut it = args.iter();

    while let Some(arg) = it.next() {
        match arg.as_str() {
            // Matched before the two catch-alls below, which would otherwise
            // try to parse it as a duration or quietly make it part of the
            // name. The cost is that a timer cannot be called `--private`.
            "--private" | "--hidden" | "-p" => private = true,
            "--in" | "-i" => {
                let spec = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--in needs a duration"))?;
                deadline = Some(timers_core::now() + timespec::parse_in(spec)?);
            }
            "--at" | "-a" => {
                let spec = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--at needs a time"))?;
                deadline = Some(timespec::parse_at(spec)?);
            }
            word if deadline.is_none() && words.is_empty() => {
                deadline = Some(timespec::parse_auto(word)?);
            }
            word => words.push(word),
        }
    }

    let deadline =
        deadline.ok_or_else(|| anyhow::anyhow!("no time given (try: timers add 25m tea)"))?;
    let name = words.join(" ");
    Ok((
        deadline,
        if name.is_empty() {
            "timer".into()
        } else {
            name
        },
        private,
    ))
}

fn remove(paths: &Paths, args: &[String]) {
    with_ids(paths, args, "rm", |id| Request::Remove { id });
}

fn ack(paths: &Paths, args: &[String]) {
    if args.iter().any(|a| a == "--all" || a == "-a" || a == "all") {
        once(paths, Request::AckAll);
        return;
    }
    with_ids(paths, args, "ack", |id| Request::Ack { id });
}

/// Send one request per id and print the last snapshot once.
///
/// Printing per id would emit three near-identical objects for `rm 1 2 3`; what
/// a caller wants is the state it ended up in.
fn with_ids(paths: &Paths, args: &[String], verb: &str, req: impl Fn(i64) -> Request) {
    let ids: Vec<i64> = match args.iter().map(|a| a.parse::<i64>()).collect() {
        Ok(ids) => ids,
        Err(_) => {
            eprintln!("timers: {verb} takes timer ids (see `timers list`)");
            std::process::exit(2);
        }
    };
    if ids.is_empty() {
        eprintln!("timers: {verb} needs at least one id");
        std::process::exit(2);
    }

    let mut last: Option<Status> = None;
    for id in ids {
        match client::request(paths, &req(id)) {
            Ok(Response::Status(status)) => last = Some(status),
            Ok(Response::Error(msg)) => {
                eprintln!("timers: {msg}");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("timers: daemon unreachable: {e:#}");
                emit_fallback();
                std::process::exit(1);
            }
        }
    }
    if let Some(status) = last {
        emit(&status);
    }
}

/// Subscribe and print a line per update, forever, reconnecting as needed.
///
/// This is what the bar runs. It must never exit on its own and never leave the
/// widget without a line to read, so an outage prints `FALLBACK` once — not once
/// per retry — and the loop keeps trying behind it. When the daemon comes back
/// the next real line replaces it, with no bar reload.
fn tail(paths: &Paths) {
    let mut said_failed = false;

    loop {
        match client::subscribe(paths) {
            Ok(mut sub) => {
                said_failed = false;
                loop {
                    match sub.recv() {
                        Ok(Response::Status(status)) => {
                            if !print_line(&serde_json::to_string(&status).unwrap_or_default()) {
                                // stdout is gone: the bar exited. Nothing left
                                // to do but leave quietly.
                                return;
                            }
                        }
                        Ok(Response::Error(msg)) => eprintln!("timers: {msg}"),
                        Err(e) => {
                            eprintln!("timers: subscription ended: {e:#}");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                if !said_failed {
                    eprintln!("timers: daemon unreachable: {e:#}");
                    if !print_line(FALLBACK) {
                        return;
                    }
                    said_failed = true;
                }
            }
        }
        std::thread::sleep(RECONNECT_DELAY);
    }
}

fn emit(status: &Status) {
    match serde_json::to_string(status) {
        Ok(line) => {
            print_line(&line);
        }
        Err(e) => {
            eprintln!("timers: could not serialize the status: {e:#}");
            emit_fallback();
        }
    }
}

fn emit_fallback() {
    print_line(FALLBACK);
}

/// Print one line and flush it. Returns false once stdout is gone.
///
/// The flush is what makes `deflisten` work: without it the line sits in a
/// buffer until the pipe fills, and the bar shows a countdown from minutes ago.
/// A broken pipe is the bar exiting, which is not an error worth reporting.
fn print_line(line: &str) -> bool {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{line}").is_ok() && out.flush().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_matches() {
        assert_eq!(
            FALLBACK,
            serde_json::to_string(&Status::failed()).unwrap(),
            "the literal the bar falls back to has drifted from Status::failed()"
        );
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn add_takes_a_spec_and_the_rest_as_a_name() {
        let now = timers_core::now();

        let (deadline, name, private) = parse_add(&args(&["25m", "green", "tea"])).unwrap();
        assert_eq!(name, "green tea", "the name does not need quoting");
        assert!((deadline - now - 1500).abs() <= 1);
        assert!(!private, "timers are visible unless asked otherwise");

        let (deadline, name, _) = parse_add(&args(&["--in", "90s", "pasta"])).unwrap();
        assert_eq!(name, "pasta");
        assert!((deadline - now - 90).abs() <= 1);

        let (deadline, name, _) = parse_add(&args(&["--at", "23:59", "standup"])).unwrap();
        assert_eq!(name, "standup");
        assert!(deadline > now);

        // A colon spec with no flag is a time of day, and a nameless timer is
        // still a timer.
        let (deadline, name, _) = parse_add(&args(&["23:59"])).unwrap();
        assert_eq!(name, "timer");
        assert!(deadline > now);
    }

    #[test]
    fn the_private_flag_never_lands_in_the_name() {
        // The trap: without an explicit arm for it, `--private` either gets
        // parsed as a duration or silently becomes part of the timer's name.
        for spec in [
            vec!["25m", "--private", "salary", "review"],
            vec!["--private", "25m", "salary", "review"],
            vec!["25m", "salary", "review", "--private"],
            vec!["--in", "25m", "--hidden", "salary", "review"],
            vec!["-p", "25m", "salary", "review"],
        ] {
            let (_, name, private) = parse_add(&args(&spec)).unwrap();
            assert_eq!(name, "salary review", "{spec:?} mangled the name");
            assert!(private, "{spec:?} did not mark it hidden");
        }
    }

    #[test]
    fn add_rejects_what_it_cannot_understand() {
        for bad in [
            vec![],
            args(&["tea"]),
            args(&["--in"]),
            args(&["--at", "nope", "x"]),
        ] {
            assert!(parse_add(&bad).is_err(), "{bad:?} should not parse");
        }
    }
}
