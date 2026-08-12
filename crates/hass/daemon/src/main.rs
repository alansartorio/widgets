//! The light pill's daemon: it owns the widget's mode, the last known lamp
//! state, and the socket the bar reads.
//!
//! Two threads and no async runtime, like `timers-daemon`. The main thread reads
//! commands; a listener thread accepts readers on the state socket and hands
//! each one the current line before adding it to the fan-out list.
//!
//! Everything that is still a literal in here — the entity, both paths, and the
//! fact that commands arrive on stdin from a `socat` the launcher pipes in — is
//! what the commits after this one are about. The split itself changes nothing
//! about what the program does.

use std::io::{BufRead, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use hass_core::api::Client;
use hass_core::lamp::{Lamp, LampState};
use hass_core::proto::Command;
use hass_core::widget::{Snapshot, Widget};

const ENTITY: &str = "light.desk_lamp";
const STATE_SOCKET: &str = "/tmp/hass-light-eww-output.sock";

/// Per-user: nothing else on the machine has any business watching this
/// session's lamp be dimmed.
const SOCKET_MODE: u32 = 0o600;

/// How long to wait between polls while a lamp that was just switched on has
/// not reported a brightness yet.
const SETTLE_POLL: Duration = Duration::from_millis(100);

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
        for client in self.clients.lock().unwrap().iter_mut() {
            let _ = client.write_all(line.as_bytes());
        }
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
    tracing::info!(entity = lamp.entity_id(), state = STATE_SOCKET, "starting");

    // widgets_ipc::bind rather than UnixListener::bind: it creates the parent
    // directory, unlinks a socket a killed daemon left behind — which makes bind
    // fail with EADDRINUSE even though nothing is listening — and sets the mode.
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

    for read in std::io::stdin().lock().lines() {
        let raw = read.context("reading stdin")?;
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

    Ok(())
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

    if let Err(e) = lamp.send(bar.lamp.call_for(change)) {
        tracing::warn!("{e:#}");
    }

    // A lamp that was just switched on reports no brightness until it has
    // reached one, so keep asking until it says it is on.
    if bar.lamp.is_settling() {
        loop {
            match lamp.state() {
                Ok(fresh) => bar.lamp = fresh,
                Err(e) => {
                    tracing::warn!("{e:#}");
                    break;
                }
            }
            if bar.lamp.status == hass_core::lamp::Status::On {
                break;
            }
            thread::sleep(SETTLE_POLL);
        }
        subscribers.publish(bar.snapshot());
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
