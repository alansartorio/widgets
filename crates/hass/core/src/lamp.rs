//! One light entity: what Home Assistant says it is doing, and how to ask it to
//! do something else.
//!
//! Everything outside this module works in percent, for both brightness and
//! colour temperature, because that is what a scroll notch and a progress bar
//! both mean. Home Assistant does not: brightness is 1–255 and colour
//! temperature is in mireds, between a per-lamp minimum and maximum it reports
//! in its attributes. Converting in both directions is most of what is here.
//!
//! The mired attributes (`color_temp`, `min_mireds`, `max_mireds`) are Home
//! Assistant's older spelling — newer versions also carry `*_kelvin` forms — and
//! they are what the instance this was written against sends.
//!
//! `lamp_simulator` used to be a module of its own for [`LampState`] and its two
//! methods. It is here: it is the lamp's state, and nothing was ever simulated.

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::api::Client;

/// Home Assistant's brightness scale. 0 is not in it — a lamp at brightness 0 is
/// a lamp that is off.
const BRIGHTNESS_MAX: f32 = 255.0;

/// The floor on the brightness percentage: 1/255, the dimmest a lamp can be
/// while still being on. Scrolling all the way down should leave it there rather
/// than switch it off behind the widget's back.
const MIN_BRIGHTNESS: f32 = 0.39;

/// The mired span one particular lamp accepts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub min: f32,
    pub max: f32,
}

impl Range {
    pub fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }

    /// Mireds → percent of this lamp's span.
    fn to_percent(self, mireds: f32) -> f32 {
        if self.max == self.min {
            return 0.0;
        }
        (mireds - self.min) / (self.max - self.min) * 100.0
    }

    /// Percent of this lamp's span → mireds.
    fn to_mireds(self, percent: f32) -> f32 {
        percent / 100.0 * (self.max - self.min) + self.min
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    On,
    Off,
}

/// The lamp as last seen, plus whatever has been asked of it since.
///
/// Kept locally and updated optimistically: a scroll notch moves this and the
/// widget re-renders from it immediately, rather than waiting out a round trip
/// to Home Assistant and then a bulb's own ramp. The next read from the API
/// overwrites it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LampState {
    pub status: Status,
    /// Percent, or `None` while the lamp is on but has not reported one yet.
    pub brightness: Option<f32>,
    /// Percent of `temperature_range`, or `None` as above.
    pub temperature: Option<f32>,
    pub temperature_range: Range,
}

/// A change to make, in the widget's terms rather than Home Assistant's.
///
/// `LampAbstractCommand` in the crate this came from, and it lived next to the
/// widget. It is the lamp's vocabulary, so it lives with the lamp, and the
/// dependency between the two modules now points one way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    TurnOn,
    TurnOff,
    DeltaBrightness(f32),
    DeltaTemperature(f32),
}

/// A `light.turn_on` / `light.turn_off` call, as the widget means it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Call {
    On,
    OnWithBrightness(f32),
    /// Absolute mireds, not percent: the conversion happens before this.
    OnWithTemperature(f32),
    Off,
}

impl Call {
    fn service(self) -> &'static str {
        match self {
            Self::Off => "turn_off",
            Self::On | Self::OnWithBrightness(_) | Self::OnWithTemperature(_) => "turn_on",
        }
    }

    /// The service call's body.
    ///
    /// Built with `json!` rather than through a `Serialize` impl. The version
    /// this replaces had four types for it — an enum with `#[serde(into = …)]`,
    /// an untagged enum, a struct of skipped Options and a wrapper with a
    /// `flatten` — to produce an object with two keys at most.
    fn body(self, entity_id: &str) -> Value {
        let mut body = json!({ "entity_id": entity_id });
        match self {
            Self::On | Self::Off => {}
            Self::OnWithBrightness(percent) => body["brightness_pct"] = json!(percent),
            Self::OnWithTemperature(mireds) => body["color_temp"] = json!(mireds),
        }
        body
    }
}

impl LampState {
    /// Apply a change locally, as if it had already landed.
    pub fn apply(&mut self, change: Change) {
        match change {
            Change::TurnOn => self.status = Status::On,
            Change::TurnOff => self.status = Status::Off,
            Change::DeltaBrightness(delta) => {
                if let Some(brightness) = &mut self.brightness {
                    *brightness = (*brightness + delta).clamp(MIN_BRIGHTNESS, 100.0);
                }
            }
            Change::DeltaTemperature(delta) => {
                if let Some(temperature) = &mut self.temperature {
                    *temperature = (*temperature + delta).clamp(0.0, 100.0);
                }
            }
        }
    }

    /// The service call a change turns into, once this state has absorbed it.
    ///
    /// Brightness and temperature come back as *absolute* values read off this
    /// state rather than as the delta, because Home Assistant has no relative
    /// form for either — and because two scroll notches arriving faster than the
    /// round trip should send the sum, not the last one twice.
    pub fn call_for(&self, change: Change) -> Call {
        match change {
            Change::TurnOn => Call::On,
            Change::TurnOff => Call::Off,
            Change::DeltaBrightness(_) => Call::OnWithBrightness(self.brightness.unwrap()),
            Change::DeltaTemperature(_) => Call::OnWithTemperature(self.mireds().unwrap()),
        }
    }

    /// The colour temperature in mireds.
    pub fn mireds(&self) -> Option<f32> {
        self.temperature
            .map(|percent| self.temperature_range.to_mireds(percent))
    }

    /// On, but not yet reporting the value the pill wants to show.
    ///
    /// This is the state a lamp is in for the moment after being switched on,
    /// and what the daemon's settle poll waits out.
    pub fn is_settling(&self) -> bool {
        self.status == Status::On && self.brightness.is_none()
    }
}

#[derive(Debug)]
pub struct Lamp {
    client: Client,
    entity_id: String,
}

impl Lamp {
    pub fn new(client: Client, entity_id: String) -> Self {
        Self { client, entity_id }
    }

    pub fn entity_id(&self) -> &str {
        &self.entity_id
    }

    /// Read the lamp's current state.
    pub fn state(&self) -> Result<LampState> {
        let data = self.client.state(&self.entity_id)?;
        tracing::debug!(entity = %self.entity_id, state = %data, "read state");
        parse_state(&data)
            .with_context(|| format!("unexpected state object for {}", self.entity_id))
    }

    pub fn send(&self, call: Call) -> Result<()> {
        tracing::info!(entity = %self.entity_id, ?call, "sending");
        self.client
            .call_service("light", call.service(), &call.body(&self.entity_id))
    }
}

/// Turn one `/api/states/<entity>` object into a [`LampState`].
fn parse_state(data: &Value) -> Result<LampState> {
    let attributes = &data["attributes"];
    let min_temp = attributes["min_mireds"].as_f64().unwrap() as f32;
    let max_temp = attributes["max_mireds"].as_f64().unwrap() as f32;
    let temperature_range = Range::new(min_temp, max_temp);

    Ok(match data["state"].as_str().context("no state field")? {
        "off" => LampState {
            status: Status::Off,
            brightness: None,
            temperature: None,
            temperature_range,
        },
        "on" => LampState {
            status: Status::On,
            brightness: Some(
                attributes["brightness"].as_f64().unwrap() as f32 * 100.0 / BRIGHTNESS_MAX,
            ),
            temperature: Some(
                temperature_range.to_percent(attributes["color_temp"].as_f64().unwrap() as f32),
            ),
            temperature_range,
        },
        other => anyhow::bail!("unexpected state {other:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Value {
        json!({
            "state": "on",
            "attributes": {
                "brightness": 255,
                "color_temp": 250,
                "min_mireds": 153,
                "max_mireds": 500,
            }
        })
    }

    #[test]
    fn brightness_comes_back_as_a_percentage_of_255() {
        assert_eq!(parse_state(&on()).unwrap().brightness, Some(100.0));
    }

    #[test]
    fn colour_temperature_comes_back_as_a_percentage_of_the_lamps_own_span() {
        let state = parse_state(&on()).unwrap();
        let percent = state.temperature.unwrap();
        assert!((percent - 27.95).abs() < 0.01, "{percent}");
        // And converts back to the mireds it came from.
        assert!((state.mireds().unwrap() - 250.0).abs() < 0.01);
    }

    #[test]
    fn an_off_lamp_reports_no_values_but_keeps_its_span() {
        let state = parse_state(&json!({
            "state": "off",
            "attributes": { "min_mireds": 153, "max_mireds": 500 }
        }))
        .unwrap();
        assert_eq!(state.status, Status::Off);
        assert_eq!(state.brightness, None);
        assert_eq!(state.temperature_range, Range::new(153.0, 500.0));
    }

    #[test]
    fn scrolling_down_stops_at_the_floor_not_at_off() {
        let mut state = parse_state(&on()).unwrap();
        for _ in 0..20 {
            state.apply(Change::DeltaBrightness(-10.0));
        }
        assert_eq!(state.brightness, Some(MIN_BRIGHTNESS));
        assert_eq!(state.status, Status::On);
    }

    #[test]
    fn a_call_carries_the_entity_and_only_the_field_it_changes() {
        assert_eq!(
            Call::OnWithBrightness(50.0).body("light.x"),
            json!({ "entity_id": "light.x", "brightness_pct": 50.0 })
        );
        assert_eq!(Call::Off.body("light.x"), json!({ "entity_id": "light.x" }));
        assert_eq!(Call::Off.service(), "turn_off");
        assert_eq!(Call::OnWithTemperature(250.0).service(), "turn_on");
    }
}
