//! claude-usage-daemon — the poll loop and socket server.
//!
//! Threads, std only:
//!   - main: a poll loop (every 5 min, or `CLAUDE_USAGE_POLL_INTERVAL` seconds)
//!     that hits the endpoint, records to the DB, and publishes the result as
//!     the new shared status.
//!   - listener: accepts connections on the control socket.
//!   - one per connection: answers `Status` and `Chart` and closes, or, for
//!     `Subscribe`, stays alive pushing a frame per poll.
//!
//! A connection gets its own thread because a subscriber holds its connection
//! open for as long as the bar runs; answering in the accept loop would mean
//! the first subscriber wedged the socket for everybody else.
//!
//! The daemon is the sole reader/writer of the DB and the token. Clients
//! (the CLI and the GUI) talk to it over the socket; they never touch files.

use std::os::unix::net::UnixStream;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use claude_usage_core::{
    api::{self, UsageStatusSnapshot},
    db,
    paths::{ClaudeUsagePaths, Paths},
    proto::{MAX_FRAME, Request, Response, SOCKET_MODE},
};
use rusqlite::Connection;

/// How often the poll loop runs when nothing overrides it.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Overrides [`POLL_INTERVAL`], in whole seconds. The NixOS module sets it
/// from `services.claude-usage.pollInterval`.
const POLL_INTERVAL_ENV: &str = "CLAUDE_USAGE_POLL_INTERVAL";
/// How long a `PollNow` waits for the poll it asked for before giving up and
/// answering with whatever the last one produced. Comfortably past the
/// endpoint timeout plus a token refresh.
const POLL_NOW_TIMEOUT: Duration = Duration::from_secs(45);
/// A client that connects and then says nothing must not hold a thread.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// The last poll's result, plus a counter that ticks on every publish.
///
/// Subscribers track the generation they last sent rather than comparing
/// snapshots, so a poll that produces identical numbers still counts as an
/// update and the bar's `fetched_at` keeps moving.
struct Status {
    generation: u64,
    snapshot: UsageStatusSnapshot,
}

struct State {
    status: Mutex<Status>,
    /// Notified whenever `status.generation` moves.
    updated: Condvar,
    /// Set by `PollNow` to cut the poll loop's sleep short.
    poll_requested: Mutex<bool>,
    wake_poll: Condvar,
    db: Mutex<Connection>,
    /// How long the poll loop sleeps between polls.
    poll_interval: Duration,
}

impl State {
    fn new(db: Connection, poll_interval: Duration) -> Self {
        Self {
            status: Mutex::new(Status {
                generation: 0,
                snapshot: UsageStatusSnapshot::default(),
            }),
            updated: Condvar::new(),
            poll_requested: Mutex::new(false),
            wake_poll: Condvar::new(),
            db: Mutex::new(db),
            poll_interval,
        }
    }

    /// Record a poll's result as the current status and wake every subscriber.
    fn publish(&self, snapshot: UsageStatusSnapshot) {
        {
            let mut status = self.status.lock().unwrap();
            status.generation += 1;
            status.snapshot = snapshot;
        }
        self.updated.notify_all();
    }

    fn generation(&self) -> u64 {
        self.status.lock().unwrap().generation
    }

    /// Block until the status is newer than `seen`, then hand back a copy of it
    /// and the generation it carries.
    fn status_after(&self, seen: u64) -> (u64, UsageStatusSnapshot) {
        let status = self.status.lock().unwrap();
        let status = self
            .updated
            .wait_while(status, |s| s.generation <= seen)
            .unwrap();
        (status.generation, status.snapshot.clone())
    }

    /// Same, but gives up after `timeout` and returns whatever is current.
    fn status_after_or_timeout(&self, seen: u64, timeout: Duration) -> UsageStatusSnapshot {
        let status = self.status.lock().unwrap();
        let (status, _) = self
            .updated
            .wait_timeout_while(status, timeout, |s| s.generation <= seen)
            .unwrap();
        status.snapshot.clone()
    }

    /// Ask the poll loop to poll now instead of sleeping out its interval.
    fn request_poll(&self) {
        *self.poll_requested.lock().unwrap() = true;
        self.wake_poll.notify_all();
    }

    /// Sleep until the interval runs out or somebody asks for a poll.
    fn sleep_until_next_poll(&self) {
        let requested = self.poll_requested.lock().unwrap();
        let (mut requested, _) = self
            .wake_poll
            .wait_timeout_while(requested, self.poll_interval, |asked| !*asked)
            .unwrap();
        *requested = false;
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let once = std::env::args().any(|a| a == "--once");
    let paths = Paths::from_env();

    let conn = db::open(&paths.db())?;

    if once {
        // One poll, forcing the refresh in case the seed token is stale.
        let outcome = api::poll(&paths.credentials(), &paths.refresh_stamp(), true);
        if let Some(reading) = &outcome.reading {
            let _ = db::record(&conn, reading);
        }
        println!("{}", serde_json::to_string(&outcome.status)?);
        return Ok(());
    }

    let poll_interval = poll_interval_from_env();
    tracing::info!("polling every {}s", poll_interval.as_secs());

    let state = Arc::new(State::new(conn, poll_interval));

    // The first poll forces the refresh too, so a freshly-started service has
    // data — and a live token — before anything connects.
    poll_and_publish(&paths, &state, true);

    let listener_state = Arc::clone(&state);
    let listener_paths = paths.clone();
    let _listener = thread::spawn(move || {
        if let Err(e) = serve(&listener_paths, &listener_state) {
            tracing::error!("listener exited: {e:#}");
        }
    });

    // Poll loop (main thread).
    loop {
        state.sleep_until_next_poll();
        poll_and_publish(&paths, &state, false);
    }
}

/// The poll interval from [`POLL_INTERVAL_ENV`], or [`POLL_INTERVAL`].
///
/// A value that isn't a positive whole number of seconds — unset, empty,
/// misspelt, zero — falls back to the default rather than failing to start:
/// a bad interval is not a reason to leave the bar without a daemon. It says
/// so in the journal, and `main` logs the interval it ended up with.
fn poll_interval_from_env() -> Duration {
    let raw = match std::env::var(POLL_INTERVAL_ENV) {
        Ok(raw) => raw,
        Err(_) => return POLL_INTERVAL,
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return POLL_INTERVAL;
    }
    match raw.parse::<u64>() {
        Ok(secs) if secs > 0 => Duration::from_secs(secs),
        _ => {
            tracing::warn!("{POLL_INTERVAL_ENV}={raw:?} is not a positive number of seconds");
            POLL_INTERVAL
        }
    }
}

/// Poll the endpoint, record what came back, and publish it to subscribers.
fn poll_and_publish(paths: &Paths, state: &Arc<State>, force_refresh: bool) {
    let outcome = api::poll(&paths.credentials(), &paths.refresh_stamp(), force_refresh);
    if let Some(reading) = &outcome.reading {
        match state.db.lock() {
            Ok(conn) => {
                if let Err(e) = db::record(&conn, reading) {
                    tracing::warn!("record: {e:#}");
                }
            }
            Err(e) => tracing::error!("db lock poisoned: {e}"),
        }
    }
    tracing::info!(
        "poll: ok={} session={}{} weekly={}",
        outcome.status.ok,
        outcome.status.session,
        if outcome.status.stale { " (stale)" } else { "" },
        outcome.status.weekly,
    );
    state.publish(outcome.status);
}

/// Accept connections forever, handing each to its own thread.
fn serve(paths: &Paths, state: &Arc<State>) -> anyhow::Result<()> {
    let listener = widgets_ipc::bind(&paths.socket(), SOCKET_MODE)?;
    tracing::info!("listening on {}", paths.socket().display());

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("accept: {e:#}");
                continue;
            }
        };
        let state = Arc::clone(state);
        thread::spawn(move || {
            if let Err(e) = handle_conn(stream, &state) {
                tracing::warn!("connection: {e:#}");
            }
        });
    }
    Ok(())
}

fn handle_conn(stream: UnixStream, state: &Arc<State>) -> anyhow::Result<()> {
    stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;

    match read_request(&stream)? {
        Request::Subscribe => stream_status(&stream, state),
        other => {
            let resp = handle(&other, state);
            write_response(&stream, &resp)
        }
    }
}

/// Push a status frame per poll for as long as the client keeps reading.
///
/// Ends when a write fails, which is how a client that went away shows up —
/// the bar exiting is the normal case, not an error worth logging loudly.
fn stream_status(stream: &UnixStream, state: &Arc<State>) -> anyhow::Result<()> {
    // We only ever write on this connection; the request timeout set for
    // reading the subscribe frame must not outlive it.
    stream.set_read_timeout(None)?;

    let mut sent = 0;
    loop {
        let (generation, snapshot) = state.status_after(sent);
        sent = generation;
        if let Err(e) = write_response(stream, &Response::Status(snapshot.into())) {
            tracing::debug!("subscriber gone: {e:#}");
            return Ok(());
        }
    }
}

fn handle(req: &Request, state: &Arc<State>) -> Response {
    match req {
        Request::Status => Response::Status(state.status.lock().unwrap().snapshot.clone().into()),
        Request::PollNow => {
            // Wait for the poll we just asked for, so the answer is the fresh
            // reading rather than the one the caller already had.
            let seen = state.generation();
            state.request_poll();
            let snapshot = state.status_after_or_timeout(seen, POLL_NOW_TIMEOUT);
            Response::Status(snapshot.into())
        }
        Request::Chart { view } => {
            let conn = state.db.lock().unwrap();
            match db::query_chart(&conn, view) {
                Ok(chart) => Response::Chart(chart),
                Err(e) => Response::Error(format!("chart: {e:#}")),
            }
        }
        // Handled by `handle_conn`, which never routes it here.
        Request::Subscribe => Response::Error("subscribe is not a one-shot request".into()),
    }
}

// `&UnixStream` implements Read and Write, which is what lets these take a
// shared reference: the subscriber loop below holds the stream while writing to
// it, and cloning the fd only to satisfy `&mut` would be busywork.
fn read_request(stream: &UnixStream) -> anyhow::Result<Request> {
    widgets_ipc::read_frame(&mut { stream }, MAX_FRAME)
}

fn write_response(stream: &UnixStream, resp: &Response) -> anyhow::Result<()> {
    widgets_ipc::write_frame(&mut { stream }, resp)
}
