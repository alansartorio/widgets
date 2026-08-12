//! The timers daemon: it owns the deadlines, the database, and the
//! acknowledgement state, and it is the only process that fires a notification.
//!
//! Threads, std only — no async runtime. The main thread runs the deadline
//! loop; a listener thread accepts connections and hands each one to a thread
//! of its own. Shared state is a `Mutex` + `Condvar` pair rather than channels,
//! because what subscribers wait on is "the snapshot moved", not a queue of
//! events they must each receive.
//!
//! The deadline loop and the 1 s display tick are the *same* wait. Countdowns
//! are shown to the second, so a tick already lands within a second of any
//! deadline; clamping the tick to the next deadline is all "sleep until the
//! next deadline" needs to be, and it buys a single wait with a single timeout
//! instead of a timer wheel. The cost is that a timer can fire up to a second
//! late, which for a named kitchen timer is invisible.

use std::os::unix::net::UnixStream;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::Connection;
use timers_core::models::{Status, Timer};
use timers_core::paths::{Paths, TimersPaths};
use timers_core::proto::{MAX_FRAME, Request, Response, SOCKET_MODE};
use timers_core::{db, now};

/// How often the daemon publishes when *nothing* is counting down.
///
/// Without this an idle daemon would go silent indefinitely, and a subscriber
/// could not tell "no timers" from "the daemon is wedged". Two frames a minute
/// is a cheap price for making silence mean something, and it is what the
/// client's watchdog is sized against.
const KEEPALIVE: Duration = Duration::from_secs(30);

/// A timer whose deadline passed longer ago than this fires silently.
///
/// Starting the daemon after the machine was off for a week should not dump a
/// week of notifications on the screen at once. The timers are still marked
/// fired and still colour the bar until acknowledged — only the popup is
/// suppressed, because a notification about an hour-old deadline is noise.
const NOTIFY_GRACE: i64 = 3600;

/// Per-connection socket deadlines, so one wedged client cannot pin a thread.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// The published snapshot and a counter that moves every time it is published.
///
/// Subscribers compare generations rather than statuses, so a tick that changes
/// nothing but the clock still counts as an update and the countdown in the bar
/// keeps moving.
struct Snapshot {
    generation: u64,
    status: Status,
}

struct State {
    /// The authoritative timer list. The database is the durable mirror: every
    /// mutation writes both under this lock, so a restart reloads exactly what
    /// was live.
    timers: Mutex<Vec<Timer>>,
    snapshot: Mutex<Snapshot>,
    /// Notified whenever `snapshot.generation` moves.
    updated: Condvar,
    /// Set by any mutation to cut the deadline loop's sleep short. A *separate*
    /// mutex and condvar from `updated` on purpose: the loop is what publishes,
    /// so if it also waited on `updated` it would wake itself forever.
    dirty: Mutex<bool>,
    wake: Condvar,
    db: Mutex<Connection>,
}

impl State {
    fn new(db: Connection, timers: Vec<Timer>) -> Self {
        let status = Status::from_timers(&timers, now());
        Self {
            timers: Mutex::new(timers),
            snapshot: Mutex::new(Snapshot {
                generation: 1,
                status,
            }),
            updated: Condvar::new(),
            dirty: Mutex::new(false),
            wake: Condvar::new(),
            db: Mutex::new(db),
        }
    }

    /// Recompute the snapshot from the timer list and wake every subscriber.
    fn publish(&self) {
        self.publish_at(now());
    }

    /// Publish as of a caller-supplied instant.
    ///
    /// The loop expires and then publishes against the *same* `now`, so the two
    /// can never disagree. Reading the clock twice let a wake that landed a
    /// hair before a second boundary expire nothing and then publish a timer
    /// sitting at `0:00` and still counting — a state that does not exist.
    fn publish_at(&self, now: i64) {
        let status = {
            let timers = self.timers.lock().unwrap();
            Status::from_timers(&timers, now)
        };
        let mut snap = self.snapshot.lock().unwrap();
        snap.generation = snap.generation.wrapping_add(1);
        snap.status = status;
        drop(snap);
        self.updated.notify_all();
    }

    /// The snapshot as of right now, without waiting for the loop to tick.
    ///
    /// One-shot clients get a freshly computed answer rather than one up to a
    /// second stale — and, more importantly, rather than one from however long
    /// ago the loop last ran when it is parked on an empty list.
    fn current(&self) -> Status {
        let timers = self.timers.lock().unwrap();
        Status::from_timers(&timers, now())
    }

    /// Block until the snapshot has been published past `seen`.
    fn status_after(&self, seen: u64) -> (u64, Status) {
        let snap = self.snapshot.lock().unwrap();
        let snap = self
            .updated
            .wait_while(snap, |s| s.generation <= seen)
            .unwrap();
        (snap.generation, snap.status.clone())
    }

    fn generation(&self) -> u64 {
        self.snapshot.lock().unwrap().generation
    }

    /// Tell the deadline loop that the timer list changed under it.
    fn mark_dirty(&self) {
        *self.dirty.lock().unwrap() = true;
        self.wake.notify_all();
    }

    /// Sleep for `d`, or until a mutation cuts it short.
    fn sleep(&self, d: Duration) {
        let dirty = self.dirty.lock().unwrap();
        let (mut dirty, _) = self
            .wake
            .wait_timeout_while(dirty, d, |asked| !*asked)
            .unwrap();
        *dirty = false;
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    if let Err(e) = run() {
        tracing::error!("fatal: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let paths = Paths::from_env();
    tracing::info!(db = %paths.db().display(), socket = %paths.socket().display(), "starting");

    let db = db::open(&paths.db())?;
    let timers = db::list(&db)?;
    tracing::info!(count = timers.len(), "loaded timers");

    let state = Arc::new(State::new(db, timers));

    // Anything already overdue fires before the socket is even up, so a client
    // that connects immediately after a restart sees the true state rather than
    // a snapshot that is about to change.
    expire(&state, now());
    state.publish();

    {
        let state = Arc::clone(&state);
        let paths = paths.clone();
        std::thread::Builder::new()
            .name("listener".into())
            .spawn(move || {
                if let Err(e) = serve(&paths, state) {
                    tracing::error!("listener died: {e:#}");
                    // Without a socket this process is useless to everyone;
                    // better to exit and let systemd restart it than to sit
                    // there counting down for nobody.
                    std::process::exit(1);
                }
            })
            .context("spawning the listener thread")?;
    }

    loop {
        state.sleep(next_wait(&state));
        let now = now();
        expire(&state, now);
        state.publish_at(now);
    }
}

/// How long to sleep before the next publish.
///
/// While the list holds anything at all, that is the next whole second.
/// Aligning to the boundary rather than sleeping a flat second keeps the
/// displayed number flipping in step with the wall clock instead of drifting a
/// few milliseconds further out on every iteration.
///
/// There is deliberately no separate "sleep until the next deadline" branch.
/// Waking every second already catches every deadline within a second, which is
/// the resolution the countdown is displayed at — clamping the wait to the
/// soonest deadline as well would be arithmetic that never changes the answer.
///
/// Note this ticks for *fired* timers too, not just pending ones. An
/// unacknowledged alarm shows how long ago it went off, and that text has to
/// keep moving; an earlier version filtered to pending timers here and froze
/// the "fired 0:07 ago" line for a whole keepalive interval.
///
/// With the list empty there is nothing to recompute, so the wait stretches to
/// the keepalive and the daemon costs two wakeups a minute.
fn next_wait(state: &State) -> Duration {
    if state.timers.lock().unwrap().is_empty() {
        KEEPALIVE
    } else {
        until_next_second()
    }
}

/// Time left in the current wall-clock second, never zero.
fn until_next_second() -> Duration {
    let subsec = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    // A zero-length wait would spin, so a boundary landed on exactly becomes a
    // full second.
    Duration::from_nanos(1_000_000_000u64.saturating_sub(subsec as u64).max(1))
}

/// Mark every timer whose deadline has passed as fired, and notify about it.
fn expire(state: &Arc<State>, now: i64) {
    // Everything that touches shared state happens here, and the locks are
    // released before a single D-Bus call is made.
    // Each fired timer as (what to announce it as, its id, its deadline). The
    // name is masked here, while the lock is still held and the real one is
    // still in reach — so nothing downstream has to remember to do it.
    let fired: Vec<(String, i64, i64)> = {
        let mut timers = state.timers.lock().unwrap();
        let db = state.db.lock().unwrap();

        let mut fired = Vec::new();
        for timer in timers.iter_mut() {
            if timer.firing() || timer.deadline > now {
                continue;
            }
            timer.fired_at = Some(now);
            if let Err(e) = db::mark_fired(&db, timer.id, now) {
                // The in-memory list is authoritative for this run, so a failed
                // write costs the fired state only across a restart. Losing the
                // notification as well would be worse.
                tracing::warn!(id = timer.id, "could not record fired state: {e:#}");
            }
            let announced = if timer.private {
                // The notification is the *loudest* place a hidden name could
                // surface: critical urgency, no timeout, and mako keeps it in
                // history. Masking the bar and not this would be a feature that
                // only looks like it works.
                timers_core::models::MASKED_NAME.to_string()
            } else {
                timer.name.clone()
            };
            fired.push((announced, timer.id, timer.deadline));
        }
        fired
    };

    // Outside the locks: a notification daemon that has wedged must not hold
    // the timer list hostage, and `notify_rust` blocks on a D-Bus round trip.
    for (name, id, deadline) in fired {
        let late = now - deadline;
        if late > NOTIFY_GRACE {
            tracing::info!(
                id,
                late,
                "deadline passed while the daemon was down; not notifying"
            );
            continue;
        }
        tracing::info!(id, %name, "timer finished");
        notify(&name);
    }
}

fn notify(name: &str) {
    let result = notify_rust::Notification::new()
        .summary(name)
        .body("timer finished")
        .icon("alarm-symbolic")
        .appname("timers")
        // Critical so it survives do-not-disturb and, in mako's defaults, does
        // not time out on its own. Acknowledging is a deliberate act — that is
        // the whole point of the widget staying coloured.
        .urgency(notify_rust::Urgency::Critical)
        .timeout(notify_rust::Timeout::Never)
        .show();

    if let Err(e) = result {
        // No notification daemon, or no session bus. The timer still fired and
        // the bar still turns red, so this is a degraded state, not a failure.
        tracing::warn!(%name, "could not show a notification: {e:#}");
    }
}

/// Bind the control socket and accept forever.
fn serve(paths: &Paths, state: Arc<State>) -> Result<()> {
    let listener = widgets_ipc::bind(&paths.socket(), SOCKET_MODE)?;
    tracing::info!(socket = %paths.socket().display(), "listening");

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("accept failed: {e:#}");
                continue;
            }
        };
        let state = Arc::clone(&state);
        // A thread per connection: there are only ever a handful (one bar, the
        // odd GUI window or shell command), and a subscriber has to block for
        // minutes at a time without stalling anyone else.
        if let Err(e) = std::thread::Builder::new()
            .name("conn".into())
            .spawn(move || handle_conn(stream, state))
        {
            tracing::warn!("could not spawn a connection thread: {e:#}");
        }
    }
    Ok(())
}

fn handle_conn(mut stream: UnixStream, state: Arc<State>) {
    if let Err(e) = stream.set_read_timeout(Some(REQUEST_TIMEOUT)) {
        tracing::debug!("setting the read timeout failed: {e:#}");
        return;
    }
    if let Err(e) = stream.set_write_timeout(Some(WRITE_TIMEOUT)) {
        tracing::debug!("setting the write timeout failed: {e:#}");
        return;
    }

    let req = match read_request(&mut stream) {
        Ok(req) => req,
        Err(e) => {
            tracing::debug!("unreadable request: {e:#}");
            return;
        }
    };

    if matches!(req, Request::Subscribe) {
        stream_status(stream, state);
        return;
    }

    let response = handle(req, &state);
    if let Err(e) = write_frame(&mut stream, &response) {
        tracing::debug!("could not answer: {e:#}");
    }
}

fn handle(req: Request, state: &Arc<State>) -> Response {
    match req {
        Request::List => Response::Status(state.current()),
        Request::Add {
            name,
            deadline,
            private,
        } => match add(state, name, deadline, private) {
            Ok(status) => Response::Status(status),
            Err(e) => Response::Error(format!("{e:#}")),
        },
        Request::Remove { id } => Response::Status(remove(state, id)),
        Request::Ack { id } => match ack(state, id) {
            Ok(status) => Response::Status(status),
            Err(e) => Response::Error(format!("{e:#}")),
        },
        Request::AckAll => Response::Status(ack_all(state)),
        // Routed before this function is reached.
        Request::Subscribe => Response::Error("subscribe is handled separately".into()),
    }
}

fn add(state: &Arc<State>, name: String, deadline: i64, private: bool) -> Result<Status> {
    let name = name.trim();
    let name = if name.is_empty() {
        timers_core::models::MASKED_NAME
    } else {
        name
    };
    // A bound rather than a schema constraint: the point is to keep one absurd
    // paste from making every tooltip unreadable, not to protect the database.
    if name.chars().count() > 200 {
        anyhow::bail!("that name is too long (200 characters at most)");
    }
    let created_at = now();
    if deadline <= created_at {
        anyhow::bail!("that deadline has already passed");
    }

    let id = {
        let db = state.db.lock().unwrap();
        db::insert(&db, name, deadline, created_at, private)?
    };
    {
        let mut timers = state.timers.lock().unwrap();
        timers.push(Timer {
            id,
            name: name.to_string(),
            deadline,
            created_at,
            fired_at: None,
            private,
        });
    }
    // A hidden timer's name stays out of the journal too. Not a screen leak like
    // the bar or a notification, but `journalctl` is a shoulder away from being
    // one and the id is enough to follow what happened.
    if private {
        tracing::info!(id, deadline, "timer added (hidden)");
    } else {
        tracing::info!(id, %name, deadline, "timer added");
    }
    Ok(settle(state))
}

fn remove(state: &Arc<State>, id: i64) -> Status {
    {
        let db = state.db.lock().unwrap();
        if let Err(e) = db::delete(&db, id) {
            tracing::warn!(id, "could not delete: {e:#}");
        }
    }
    // An unknown id is not an error: two clients racing to remove the same
    // timer should both come away happy.
    state.timers.lock().unwrap().retain(|t| t.id != id);
    tracing::info!(id, "timer removed");
    settle(state)
}

fn ack(state: &Arc<State>, id: i64) -> Result<Status> {
    {
        let timers = state.timers.lock().unwrap();
        match timers.iter().find(|t| t.id == id) {
            // Refusing rather than silently deleting: acknowledging something
            // that has not gone off is almost certainly a mistyped id, and
            // `rm` is right there for cancelling.
            Some(t) if !t.firing() => anyhow::bail!("timer {id} has not gone off yet (use rm)"),
            Some(_) => {}
            None => anyhow::bail!("no timer with id {id}"),
        }
    }
    Ok(remove(state, id))
}

fn ack_all(state: &Arc<State>) -> Status {
    {
        let db = state.db.lock().unwrap();
        if let Err(e) = db::delete_fired(&db) {
            tracing::warn!("could not clear fired timers: {e:#}");
        }
    }
    let cleared = {
        let mut timers = state.timers.lock().unwrap();
        let before = timers.len();
        timers.retain(|t| !t.firing());
        before - timers.len()
    };
    tracing::info!(cleared, "acknowledged");
    settle(state)
}

/// Publish after a mutation, wake the deadline loop, and hand back the snapshot
/// the caller should be told about.
///
/// Waking the loop matters most when it is parked: adding a five-second timer
/// while nothing else was pending must not wait out a keepalive.
fn settle(state: &Arc<State>) -> Status {
    state.publish();
    state.mark_dirty();
    state.snapshot.lock().unwrap().status.clone()
}

/// Push a frame to this subscriber every time the snapshot moves.
fn stream_status(mut stream: UnixStream, state: Arc<State>) {
    // Write-only from here on: the subscriber has said everything it is going
    // to say, and a read deadline would only fire on an idle connection.
    if let Err(e) = stream.set_read_timeout(None) {
        tracing::debug!("clearing the read timeout failed: {e:#}");
    }

    // Start one behind the current generation so the first send is immediate:
    // a bar that just started has nothing on screen and should not wait a tick.
    let mut seen = state.generation().saturating_sub(1);
    loop {
        let (generation, status) = state.status_after(seen);
        seen = generation;
        if let Err(e) = write_frame(&mut stream, &Response::Status(status)) {
            // A bar exiting or an eww reload looks exactly like this, so it is
            // normal traffic rather than a warning.
            tracing::debug!("subscriber gone: {e:#}");
            return;
        }
    }
}

fn read_request(stream: &mut UnixStream) -> Result<Request> {
    widgets_ipc::read_frame(stream, MAX_FRAME)
}

fn write_frame(stream: &mut UnixStream, response: &Response) -> Result<()> {
    widgets_ipc::write_frame(stream, response)
}
