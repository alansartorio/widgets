//! Threads and std, no async runtime — which is what every other daemon in this
//! workspace does, and what this program's actual concurrency needs.
//!
//! There were three jobs under tokio and there are three here: read commands
//! from stdin, accept readers on the output socket, and write the widget's state
//! to every one of them. One lamp, one HTTP request at a time, and a scroll
//! event every few seconds at the very most — nothing to overlap, and nothing
//! that an executor was buying. What it cost was tokio, tokio-stream, reqwest,
//! hyper and rustls on every cold build.
//!
//! Behaviour is unchanged, including the things worth changing: the lamp is
//! still `light.desk_lamp`, the socket paths are still under `/tmp`, the step
//! is still ten percent, commands still arrive on stdin from a `socat` the
//! launcher pipes in. Those come after.
//!
//! The one structural difference is the lock. Under tokio the widget state and
//! the lamp state were two `Mutex`es taken in a fixed order; they are one here,
//! because they only ever change together and a single lock cannot be taken in
//! the wrong order.

use std::io::{BufRead, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::hass_client::HassClient;
use crate::ipc_protocol::Message;
use crate::lamp::{Lamp, LampCommand};
use crate::lamp_simulator::{LampState, LampStatus};
use crate::widget_state::{LampAbstractCommand, WidgetOnSubState, WidgetState};

mod hass_client;
mod ipc_protocol;
mod lamp;
mod lamp_simulator;
mod widget_state;

static OUTPUT_SOCKET_PATH: &str = "/tmp/hass-light-eww-output.sock";

/// How long to wait between polls while a lamp that was just switched on has
/// not reported a brightness yet.
const SETTLE_POLL: Duration = Duration::from_millis(100);

/// The widget and the lamp, which only ever change together.
struct Bar {
    widget: WidgetState,
    lamp: LampState,
}

/// Everyone reading the output socket.
struct Clients(Mutex<Vec<UnixStream>>);

impl Clients {
    fn publish(&self, bar: &Bar) {
        let mut line = serde_json::to_string(&bar.widget.with_data(&bar.lamp))
            .expect("widget data is always serialisable");
        line.push('\n');

        for client in self.0.lock().unwrap().iter_mut() {
            let _ = client.write_all(line.as_bytes());
        }
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
    tracing::info!("starting");

    let host = std::env::var("HASS_HOST").context("please set up the HASS_HOST env variable")?;
    let token = std::env::var("HASS_TOKEN").context("please set up the HASS_TOKEN env variable")?;

    let mut lamp = Lamp::new(
        HassClient::new(&host, token),
        "light.desk_lamp".to_owned(),
    );
    let initial = lamp.get_state();

    let bar = Arc::new(Mutex::new(Bar {
        widget: match initial.status {
            LampStatus::On => WidgetState::On(WidgetOnSubState::Brightness),
            LampStatus::Off => WidgetState::Off,
        },
        lamp: initial,
    }));
    let clients = Arc::new(Clients(Mutex::new(Vec::new())));

    // A socket left behind by a killed process makes bind() fail with
    // EADDRINUSE even though nothing is listening.
    let _ = std::fs::remove_file(OUTPUT_SOCKET_PATH);
    let listener = UnixListener::bind(OUTPUT_SOCKET_PATH)
        .with_context(|| format!("binding {OUTPUT_SOCKET_PATH}"))?;

    thread::spawn({
        let bar = Arc::clone(&bar);
        let clients = Arc::clone(&clients);
        move || accept_loop(listener, bar, clients)
    });

    clients.publish(&bar.lock().unwrap());

    for line in std::io::stdin().lock().lines() {
        let line = line.context("reading stdin")?;
        let message: Message = serde_plain::from_str(line.trim())?;

        let mut bar = bar.lock().unwrap();

        if let Some(command) = bar.widget.apply(&message) {
            bar.lamp.apply_abstract_command(command);
            lamp.send_command(match command {
                LampAbstractCommand::TurnOn => LampCommand::On,
                LampAbstractCommand::TurnOff => LampCommand::Off,
                LampAbstractCommand::DeltaBrightness(_) => {
                    LampCommand::OnWithBrightness(bar.lamp.brightness.unwrap())
                }
                LampAbstractCommand::DeltaTemperature(_) => {
                    LampCommand::OnWithTemperature(bar.lamp.get_fixed_temperature().unwrap())
                }
            });
        }
        clients.publish(&bar);

        // A lamp that was just switched on reports no brightness until it has
        // reached one, so keep asking until it says it is on.
        if bar.lamp.status == LampStatus::On && bar.lamp.brightness.is_none() {
            loop {
                bar.lamp = lamp.get_state();
                if bar.lamp.status == LampStatus::On {
                    break;
                }
                thread::sleep(SETTLE_POLL);
            }
            clients.publish(&bar);
        }
    }

    Ok(())
}

/// Accept readers, answering each with the current line immediately: the bar
/// must not wait for the next scroll to have something to show.
fn accept_loop(listener: UnixListener, bar: Arc<Mutex<Bar>>, clients: Arc<Clients>) {
    for client in listener.incoming() {
        let mut client = match client {
            Ok(client) => client,
            Err(e) => {
                tracing::warn!("accept failed: {e}");
                continue;
            }
        };

        let bar = bar.lock().unwrap();
        let mut line = serde_json::to_string(&bar.widget.with_data(&bar.lamp))
            .expect("widget data is always serialisable");
        line.push('\n');

        let _ = client.write_all(line.as_bytes());
        clients.0.lock().unwrap().push(client);
    }
}

#[test]
fn feature() {
    let mut widget_state = WidgetState::On(WidgetOnSubState::Brightness);

    assert!(matches!(
        widget_state.apply(&Message::Increase),
        Some(LampAbstractCommand::DeltaBrightness(_))
    ));

    assert!(widget_state.apply(&Message::ToggleMode).is_none());

    assert!(matches!(
        widget_state.apply(&Message::Increase),
        Some(LampAbstractCommand::DeltaTemperature(_))
    ));

    assert!(matches!(
        widget_state.apply(&Message::ToggleState),
        Some(LampAbstractCommand::TurnOff)
    ));

    assert!(matches!(
        widget_state.apply(&Message::ToggleState),
        Some(LampAbstractCommand::TurnOn)
    ));
}
