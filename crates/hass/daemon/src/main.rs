//! The light pill's daemon: it owns the widget's mode, the last known lamp
//! state, and the two sockets the bar talks to.
//!
//! Two threads and no async runtime, like `timers-daemon`. The main thread blocks
//! on the command socket; a listener thread accepts subscribers on the state
//! socket and hands each one the current line before adding it to the fan-out
//! list.
//!
//! ```text
//!  eww  ──socat UNIX-SENDTO──▶  the command socket  (datagram)   one word per send
//!  eww  ◀──socat UNIX-CLIENT──  the state socket    (stream)     one JSON line per change
//! ```
//!
//! The entity and both paths are still literals here; that is the next commit.

use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use hass_core::api::Client;
use hass_core::lamp::{Lamp, LampState};
use hass_core::proto::Command;
use hass_core::widget::{Snapshot, Widget};

const ENTITY: &str = "light.desk_lamp";
const COMMAND_SOCKET: &str = "/tmp/hass-light.sock";
const STATE_SOCKET: &str = "/tmp/hass-light-eww-output.sock";

/// The longest datagram worth reading. The longest command is `toggle_state`;
/// anything past this is truncated and then fails to parse, which is the right
/// answer for a sender this program does not recognise.
const MAX_COMMAND: usize = 64;

/// Per-user: nothing else on the machine has any business watching this
/// session's lamp be dimmed.
const SOCKET_MODE: u32 = 0o600;

/// How long to wait between polls while a lamp that was just switched on has
/// not reported a brightness yet.
const SETTLE_POLL: Duration = Duration::from_millis(100);

/// How long to keep asking before giving up and leaving the pill on `loading`
/// until the next command.
///
/// There was no ceiling here, and the loop's exit condition was the lamp
/// reporting itself on — so a bulb that answers "off" because it never came up,
/// or one behind a Home Assistant that stops answering mid-poll, meant ten
/// requests a second for as long as the daemon lived.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(3);

/// The widget and the lamp, which only ever change together.
struct Bar {
    widget: Widget,
    lamp: LampState,
}

impl Bar {
    fn snapshot(&self) -> Snapshot {
        self.widget.snapshot(&self.lamp)
    }
}

/// Everyone currently reading the state socket.
///
/// A write that fails drops the client. That is not a refinement: every accepted
/// stream used to be pushed onto this list and its write errors ignored forever,
/// so each eww reload left a dead connection behind and the list grew for as long
/// as the process lived — which for this daemon is weeks.
#[derive(Default)]
struct Subscribers {
    clients: Mutex<Vec<UnixStream>>,
}

impl Subscribers {
    fn add(&self, client: UnixStream) {
        self.clients.lock().unwrap().push(client);
    }

    fn publish(&self, snapshot: Snapshot) {
        let line = line(snapshot);
        self.clients
            .lock()
            .unwrap()
            .retain_mut(|client| client.write_all(line.as_bytes()).is_ok());
    }
}

fn line(snapshot: Snapshot) -> String {
    let mut line = serde_json::to_string(&snapshot).expect("a snapshot is always serialisable");
    line.push('\n');
    line
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
    let host = std::env::var("HASS_HOST").context("please set up the HASS_HOST env variable")?;
    let token = std::env::var("HASS_TOKEN").context("please set up the HASS_TOKEN env variable")?;

    let lamp = Lamp::new(Client::new(&host, token), ENTITY.to_owned());
    tracing::info!(
        entity = lamp.entity_id(),
        commands = COMMAND_SOCKET,
        state = STATE_SOCKET,
        "starting"
    );

    // Both sockets before the first request, so a Home Assistant that is not up
    // yet cannot leave the bar's `socat` retrying against a path that does not
    // exist. This process exits on that error and its unit restarts it; the paths
    // existing in the meantime is what keeps the failure to one pill.
    //
    // widgets_ipc rather than std: it creates the parent directory, unlinks a
    // socket a killed daemon left behind — which makes bind fail with EADDRINUSE
    // even though nothing is listening — and sets the mode.
    let commands = widgets_ipc::bind_datagram(std::path::Path::new(COMMAND_SOCKET), SOCKET_MODE)?;
    let listener = widgets_ipc::bind(std::path::Path::new(STATE_SOCKET), SOCKET_MODE)?;

    let initial = lamp.state()?;
    let bar = Arc::new(Mutex::new(Bar {
        widget: Widget::new(&initial),
        lamp: initial,
    }));
    let subscribers = Arc::new(Subscribers::default());

    thread::spawn({
        let bar = Arc::clone(&bar);
        let subscribers = Arc::clone(&subscribers);
        move || accept_loop(listener, bar, subscribers)
    });

    subscribers.publish(bar.lock().unwrap().snapshot());

    let mut buf = [0u8; MAX_COMMAND];
    loop {
        let read = commands
            .recv(&mut buf)
            .with_context(|| format!("reading {COMMAND_SOCKET}"))?;

        let raw = String::from_utf8_lossy(&buf[..read]);
        let command = match Command::parse(&raw) {
            Ok(command) => command,
            Err(e) => {
                // Not fatal: something sent a word this does not know, and the
                // pill should keep working.
                tracing::warn!("{e:#}");
                continue;
            }
        };

        handle(command, &lamp, &bar, &subscribers);
    }
}

/// Fold one command in, send it on, and publish what the pill now shows.
///
/// The order matters: the local state absorbs the change and the bar is told
/// *before* the HTTP request goes out, so a scroll moves the pill at the speed of
/// the scroll rather than at the speed of the network. A failed request means the
/// pill is briefly optimistic about a lamp that did not move, which the next read
/// corrects — the alternative is a widget that feels broken every time.
fn handle(command: Command, lamp: &Lamp, bar: &Mutex<Bar>, subscribers: &Subscribers) {
    let mut bar = bar.lock().unwrap();

    let Some(change) = bar.widget.apply(command) else {
        // A mode toggle, or a scroll on an off pill. Publish anyway: the first
        // changes what the pill shows without touching the lamp.
        subscribers.publish(bar.snapshot());
        return;
    };

    bar.lamp.apply(change);
    subscribers.publish(bar.snapshot());

    if let Some(call) = bar.lamp.call_for(change)
        && let Err(e) = lamp.send(call)
    {
        tracing::warn!("{e:#}");
    }

    // A lamp that was just switched on reports no brightness until it has
    // reached one, so the pill says `loading` until a read comes back with a
    // number.
    if bar.lamp.is_settling() {
        settle(lamp, &mut bar, subscribers);
    }
}

/// Re-read the lamp until it reports the value the pill wants, or until
/// [`SETTLE_TIMEOUT`].
fn settle(lamp: &Lamp, bar: &mut Bar, subscribers: &Subscribers) {
    let deadline = Instant::now() + SETTLE_TIMEOUT;

    while bar.lamp.is_settling() {
        match lamp.state() {
            Ok(fresh) => {
                bar.lamp = fresh;
                subscribers.publish(bar.snapshot());
            }
            Err(e) => {
                tracing::warn!("{e:#}");
                break;
            }
        }

        if !bar.lamp.is_settling() || Instant::now() >= deadline {
            break;
        }
        thread::sleep(SETTLE_POLL);
    }
}

/// Accept readers, answering each with the current line immediately.
///
/// The bar must not wait for the next scroll to have something to show, and eww
/// keeps the last value a `deflisten` gave it — so this one line is what a
/// reconnect after an eww reload gets its pill back from.
fn accept_loop(listener: UnixListener, bar: Arc<Mutex<Bar>>, subscribers: Arc<Subscribers>) {
    for client in listener.incoming() {
        let mut client = match client {
            Ok(client) => client,
            Err(e) => {
                tracing::warn!("accept failed: {e}");
                continue;
            }
        };

        let snapshot = bar.lock().unwrap().snapshot();
        if client.write_all(line(snapshot).as_bytes()).is_ok() {
            subscribers.add(client);
        }
    }
}
