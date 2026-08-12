use std::{
    env::var,
    fs,
    path::Path,
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::{
    io::{stdin, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixListener,
    sync::Mutex,
    time::sleep,
};
use tokio_stream::{wrappers::UnixListenerStream, StreamExt};
use tracing_subscriber::EnvFilter;

use crate::{
    hass_client::HassClient,
    ipc_protocol::Message,
    lamp::{Lamp, LampCommand},
    lamp_simulator::{LampState, LampStatus},
    widget_state::{LampAbstractCommand, WidgetOnSubState, WidgetState},
};

mod hass_client;
mod ipc_protocol;
mod lamp;
mod lamp_simulator;
mod widget_state;

static TOKEN: LazyLock<String> = LazyLock::new(|| {
    var("HASS_TOKEN").expect("please set up the HASS_TOKEN env variable before running this")
});
static HOST: LazyLock<String> = LazyLock::new(|| {
    var("HASS_HOST").expect("please set up the HASS_HOST env variable before running this")
});

static OUTPUT_SOCKET_PATH: &str = "/tmp/hass-light-eww-output.sock";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    tracing::info!("starting");

    let client = HassClient::new(HOST.parse()?, TOKEN.to_owned());
    let mut lamp = Lamp::new(client, "light.desk_lamp".to_owned());
    let lamp_simulator = Arc::new(tokio::sync::Mutex::new(lamp.get_state().await));

    let widget_state = Arc::new(tokio::sync::Mutex::new(
        match lamp_simulator.lock().await.status {
            LampStatus::On => WidgetState::On(WidgetOnSubState::Brightness),
            LampStatus::Off => WidgetState::Off,
        },
    ));

    let clients = Arc::new(Mutex::new(vec![]));

    let socket = Path::new(OUTPUT_SOCKET_PATH);

    // Delete old socket if necessary
    if socket.exists() {
        fs::remove_file(socket).unwrap();
    }
    let mut stream = UnixListenerStream::new(UnixListener::bind(socket).unwrap());

    tokio::spawn({
        let clients = clients.clone();
        let widget_state = widget_state.clone();
        let lamp_simulator = lamp_simulator.clone();
        async move {
            while let Some(client) = stream.next().await {
                let widget_state = widget_state.lock().await;
                let lamp_simulator = lamp_simulator.lock().await;
                let mut client = client.unwrap();

                let mut response =
                    serde_json::to_string(&widget_state.with_data(&lamp_simulator)).unwrap();
                response.push('\n');

                let _ = client.write_all(response.as_bytes()).await;
                clients.lock().await.push(client);
            }
        }
    });

    tokio::spawn(async move {
        let print_widget_data = {
            let clients = &clients;
            move |widget_state: WidgetState, lamp_simulator: LampState| async move {
                let mut response =
                    serde_json::to_string(&widget_state.with_data(&lamp_simulator)).unwrap();
                response.push('\n');

                for client in clients.lock().await.iter_mut() {
                    let _ = client.write_all(response.as_bytes()).await;
                }
            }
        };

        {
            let widget_state = widget_state.lock().await;
            let lamp_simulator = lamp_simulator.lock().await;
            print_widget_data(*widget_state, *lamp_simulator).await;
        }

        let mut lines = BufReader::new(stdin()).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            let message: Message = serde_plain::from_str(&line).unwrap();
            let mut lamp_simulator = lamp_simulator.lock().await;
            let mut widget_state = widget_state.lock().await;

            if let Some(command) = widget_state.apply(&message) {
                lamp_simulator.apply_abstract_command(command);
                lamp.send_command(match command {
                    LampAbstractCommand::TurnOn => LampCommand::On,
                    LampAbstractCommand::TurnOff => LampCommand::Off,
                    LampAbstractCommand::DeltaBrightness(_) => {
                        LampCommand::OnWithBrightness(lamp_simulator.brightness.unwrap())
                    }
                    LampAbstractCommand::DeltaTemperature(_) => LampCommand::OnWithTemperature(
                        lamp_simulator.get_fixed_temperature().unwrap(),
                    ),
                })
                .await;
            }
            print_widget_data(*widget_state, *lamp_simulator).await;

            if lamp_simulator.status == LampStatus::On
                && (lamp_simulator.brightness.is_none() || lamp_simulator.brightness.is_none())
            {
                loop {
                    *lamp_simulator = lamp.get_state().await;
                    if lamp_simulator.status == LampStatus::On {
                        break;
                    }
                    sleep(Duration::from_secs_f32(0.1)).await;
                }
                print_widget_data(*widget_state, *lamp_simulator).await;
            }
        }
    })
    .await
    .unwrap();

    Ok(())
}

#[test]
fn feature() {
    let mut widget_state = WidgetState::On(WidgetOnSubState::Brightness);

    assert!(matches!(
        widget_state.apply(&Message::Increase),
        Some(LampAbstractCommand::DeltaBrightness(_))
    ));

    assert!(matches!(widget_state.apply(&Message::ToggleMode), None));

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
