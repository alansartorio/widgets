use crate::widget_state::LampAbstractCommand;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    min: f32,
    max: f32,
}

impl Range {
    pub fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LampState {
    pub brightness: Option<f32>,
    pub temperature: Option<f32>,
    pub temperature_range: Range,
    pub status: LampStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LampStatus {
    On,
    Off,
}

impl LampState {
    pub fn apply_abstract_command(&mut self, command: LampAbstractCommand) {
        match command {
            LampAbstractCommand::TurnOff => self.status = LampStatus::Off,
            LampAbstractCommand::TurnOn => self.status = LampStatus::On,
            LampAbstractCommand::DeltaBrightness(delta) => {
                if let Some(ref mut brightness) = self.brightness {
                    *brightness = (*brightness + delta).clamp(0.39, 100.0)
                }
            }
            LampAbstractCommand::DeltaTemperature(delta) => {
                if let Some(ref mut temperature) = self.temperature {
                    *temperature = (*temperature + delta).clamp(0.0, 100.0)
                }
            }
        }
    }

    pub fn get_fixed_temperature(&self) -> Option<f32> {
        let (min_temp, max_temp) = (self.temperature_range.min, self.temperature_range.max);
        self.temperature
            .map(|v| v / 100.0 * (max_temp - min_temp) + min_temp)
    }
}
