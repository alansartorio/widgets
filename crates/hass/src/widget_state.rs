use serde::Serialize;

use crate::{ipc_protocol::Message, lamp_simulator::LampState};

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum WidgetData {
    On(OnState),
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum OnState {
    Brightness(f32),
    Temperature(f32),
    Loading,
}

#[derive(Debug, Clone, Copy)]
pub enum WidgetState {
    On(WidgetOnSubState),
    Off,
}

#[derive(Debug, Clone, Copy)]
pub enum WidgetOnSubState {
    Brightness,
    Temperature,
}

#[derive(Debug, Clone, Copy)]
pub enum LampAbstractCommand {
    TurnOff,
    TurnOn,
    DeltaBrightness(f32),
    DeltaTemperature(f32),
}

impl WidgetState {
    pub fn apply(&mut self, message: &Message) -> Option<LampAbstractCommand> {
        use LampAbstractCommand::*;

        match message {
            Message::Increase | Message::Decrease => {
                let &mut WidgetState::On(ref mut on_state) = self else {
                    return None;
                };
                let delta = match message {
                    Message::Increase => 10.0,
                    Message::Decrease => -10.0,
                    _ => unreachable!(),
                };
                Some(match on_state {
                    WidgetOnSubState::Brightness => DeltaBrightness(delta),
                    WidgetOnSubState::Temperature => DeltaTemperature(delta),
                })
            }
            Message::ToggleMode => {
                let &mut WidgetState::On(ref mut on_state) = self else {
                    return None;
                };

                *on_state = match on_state {
                    WidgetOnSubState::Brightness => WidgetOnSubState::Temperature,
                    WidgetOnSubState::Temperature => WidgetOnSubState::Brightness,
                };

                None
            }
            Message::ToggleState => Some(match self {
                Self::On(_) => {
                    *self = Self::Off;
                    TurnOff
                }
                Self::Off => {
                    *self = Self::On(WidgetOnSubState::Brightness);
                    TurnOn
                }
            }),
        }
    }

    pub fn with_data(&self, data: &LampState) -> WidgetData {
        match self {
            Self::On(on_data) => WidgetData::On(match on_data {
                WidgetOnSubState::Brightness => {
                    if let Some(brightness) = data.brightness {
                        OnState::Brightness(brightness)
                    } else {
                        OnState::Loading
                    }
                }
                WidgetOnSubState::Temperature => {
                    if let Some(temperature) = data.temperature {
                        OnState::Temperature(temperature)
                    } else {
                        OnState::Loading
                    }
                }
            }),
            Self::Off => WidgetData::Off,
        }
    }
}
