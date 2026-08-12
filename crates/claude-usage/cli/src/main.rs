//! claude-usage — the eww pill's data source.
//!
//! A thin socket client. Three modes, all printing the same JSON shape the old
//! `get-claude-usage` script emitted, so the eww widget's `?.field` access
//! keeps working unchanged:
//!
//!   - `status` (the default) — print the daemon's current status once.
//!   - `tail` — print a line now and another every time the daemon polls, and
//!     keep the process alive. This is what the bar reads through `deflisten`:
//!     the daemon pushes, so the pill updates the moment a poll lands instead
//!     of drifting up to five minutes behind its own timer.
//!   - `refresh` — ask the daemon to poll now and print what it got. The bar's
//!     retry click; every `tail` sees the same update pushed to it.
//!
//! If the daemon isn't running (socket missing, connect refused, anything
//! else), it prints a safe fallback so the pill renders a "usage script
//! failed" message rather than disappearing entirely. `tail` keeps retrying
//! behind that fallback, so the pill heals itself once the daemon is back.

use std::io::Write;
use std::time::Duration;

use claude_usage_core::{
    client,
    models::UsageStatus,
    paths::{ClaudeUsagePaths, Paths},
    proto::{Request, Response},
};

/// What the pill reads when there is no daemon to ask.
///
/// A literal rather than `serde_json::to_string(&UsageStatus { .. })` on
/// purpose: this is the line that has to come out when everything else has
/// already gone wrong, and it can't be the thing that fails too. It must stay
/// byte-identical to what serialising the equivalent struct would produce,
/// because the eww widget's `?.field` access has to keep working precisely
/// when the daemon is down — and serde emits fields in declaration order, so
/// reordering or adding one in [`UsageStatus`] silently breaks that.
/// `fallback_matches_struct` below is what keeps the two in step; note
/// `doCheck = false` in flake.nix, so it runs in the devshell (`cargo test`),
/// not in the nix build.
const FALLBACK: &str = r#"{"ok":false,"stale":false,"expired":false,"failed":true,"configured":true,"fetched_at":"","status":0,"session":0,"weekly":0,"session_reset":"","weekly_reset":""}"#;

/// How long `tail` waits before dialling a daemon that wasn't there.
const RECONNECT_DELAY: Duration = Duration::from_secs(10);

fn main() {
    let paths = Paths::from_env();
    // `status` is also the no-argument default, which is what the bar used to
    // call and what anything else scripted against this binary expects.
    match std::env::args().nth(1).as_deref() {
        Some("tail") | Some("--tail") => tail(&paths),
        Some("refresh") | Some("--refresh") => once(&paths, Request::PollNow),
        None | Some("status") | Some("--status") => once(&paths, Request::Status),
        Some(other) => {
            eprintln!("claude-usage: unknown argument {other:?} (want status, tail, or refresh)");
            emit_fallback();
            std::process::exit(2);
        }
    }
}

/// Ask once, print once.
fn once(paths: &Paths, req: Request) {
    match client::request(paths, &req) {
        Ok(Response::Status(status)) => emit(&status),
        Ok(other) => {
            eprintln!("claude-usage: unexpected response: {other:?}");
            emit_fallback();
        }
        Err(e) => {
            eprintln!("claude-usage: daemon unreachable: {e:#}");
            emit_fallback();
        }
    }
}

/// Subscribe and print a line per update, forever.
///
/// Reconnects on its own rather than exiting, so a daemon restart is a blip in
/// the pill instead of a dead `deflisten` that only a bar reload would revive.
/// The failure line is printed once per outage, not once per retry — the pill
/// only needs telling that it is stale, and repeating it every ten seconds
/// would churn eww for nothing.
fn tail(paths: &Paths) -> ! {
    let mut said_failed = false;
    loop {
        match client::subscribe(paths) {
            Ok(mut subscription) => loop {
                match subscription.recv() {
                    Ok(Response::Status(status)) => {
                        said_failed = false;
                        emit(&status);
                    }
                    Ok(other) => {
                        eprintln!("claude-usage: unexpected frame: {other:?}");
                    }
                    Err(e) => {
                        eprintln!("claude-usage: subscription ended: {e:#}");
                        break;
                    }
                }
            },
            Err(e) => eprintln!("claude-usage: daemon unreachable: {e:#}"),
        }

        if !said_failed {
            emit_fallback();
            said_failed = true;
        }
        std::thread::sleep(RECONNECT_DELAY);
    }
}

/// One JSON object, one line, flushed — a `deflisten` reader is waiting on it.
fn emit(status: &UsageStatus) {
    match serde_json::to_string(status) {
        Ok(json) => print_line(&json),
        Err(e) => {
            eprintln!("claude-usage: serialize: {e:#}");
            emit_fallback();
        }
    }
}

fn emit_fallback() {
    print_line(FALLBACK);
}

fn print_line(line: &str) {
    let mut out = std::io::stdout().lock();
    // A broken pipe means whatever was reading us is gone; there is nothing
    // useful to say about it on the way out.
    if writeln!(out, "{line}").is_ok() {
        let _ = out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fallback line must be exactly what serialising the equivalent
    /// status produces — see the comment on [`FALLBACK`].
    #[test]
    fn fallback_matches_struct() {
        let expected = serde_json::to_string(&UsageStatus {
            // Nothing was fetched, so the numbers and timestamps are empty;
            // `failed` is the bar's cue that the CLI couldn't reach the
            // daemon, and `configured` stays true so the pill still renders
            // (a false there hides it entirely).
            ok: false,
            stale: false,
            expired: false,
            failed: true,
            configured: true,
            fetched_at: String::new(),
            status: 0,
            session: 0,
            weekly: 0,
            session_reset: String::new(),
            weekly_reset: String::new(),
        })
        .unwrap();
        assert_eq!(
            FALLBACK, expected,
            "the FALLBACK literal has drifted from UsageStatus: a field was \
             added, removed, renamed, or reordered, or a default changed. \
             serde emits fields in declaration order, so copy the `expected` \
             side above into the literal — the bar reads it when the daemon \
             is down and would lose the fields that moved."
        );
    }
}
