use serde::Serialize;

use crate::{
    hass_client::HassClient,
    lamp_simulator::{LampState, LampStatus, Range},
};

#[derive(Debug)]
pub struct Lamp {
    client: HassClient,
    entity_id: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(into = "LampCommandData")]
pub enum LampCommand {
    On,
    OnWithBrightness(f32),
    OnWithTemperature(f32),
    Off,
}

impl From<LampCommand> for LampCommandData {
    fn from(val: LampCommand) -> Self {
        match val {
            LampCommand::On => LampCommandData::On(TurnOnCommandData {
                brightness_percent: None,
                color_temp: None,
            }),
            LampCommand::OnWithBrightness(brightness_percent) => {
                LampCommandData::On(TurnOnCommandData {
                    brightness_percent: Some(brightness_percent),
                    color_temp: None,
                })
            }
            LampCommand::Off => LampCommandData::Off,
            LampCommand::OnWithTemperature(temperature) => LampCommandData::On(TurnOnCommandData {
                brightness_percent: None,
                color_temp: Some(temperature),
            }),
        }
    }
}

#[derive(Serialize)]
struct TurnOnCommandData {
    #[serde(rename = "brightness_pct", skip_serializing_if = "Option::is_none")]
    brightness_percent: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    color_temp: Option<f32>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum LampCommandData {
    On(TurnOnCommandData),
    Off,
}

#[derive(Serialize)]
struct CompleteLampCommand {
    entity_id: String,
    #[serde(flatten)]
    command: LampCommand,
}

impl Lamp {
    pub fn new(client: HassClient, entity_id: String) -> Self {
        Self { client, entity_id }
    }

    #[tracing::instrument]
    pub async fn send_command(&mut self, command: LampCommand) {
        tracing::info!("sending command to lamp");
        self.client
            .set_state(
                "light",
                {
                    use LampCommand::*;
                    match &command {
                        On | OnWithBrightness { .. } | OnWithTemperature { .. } => "turn_on",
                        Off => "turn_off",
                    }
                },
                serde_json::to_value(CompleteLampCommand {
                    command,
                    entity_id: self.entity_id.clone(),
                })
                .unwrap(),
            )
            .await
            .unwrap();
    }

    pub async fn get_state(&self) -> LampState {
        tracing::info!("obtaining state from lamp");
        let data = self.client.get_state(&self.entity_id).await.unwrap();
        tracing::debug!(
            data = serde_json::to_string(&data).unwrap(),
            "got state from lamp"
        );
        let attributes = &data["attributes"];
        let min_temp = attributes["min_mireds"].as_f64().unwrap() as f32;
        let max_temp = attributes["max_mireds"].as_f64().unwrap() as f32;
        match data["state"].as_str().unwrap() {
            "off" => LampState {
                brightness: None,
                temperature: None,
                status: LampStatus::Off,
                temperature_range: Range::new(min_temp, max_temp),
            },
            "on" => LampState {
                brightness: Some(attributes["brightness"].as_f64().unwrap() as f32 * 100.0 / 255.0),
                temperature: Some(
                    (attributes["color_temp"].as_f64().unwrap() as f32 - min_temp)
                        / (max_temp - min_temp)
                        * 100.0,
                ),
                status: LampStatus::On,
                temperature_range: Range::new(min_temp, max_temp),
            },
            _ => unreachable!(),
        }
    }
}
