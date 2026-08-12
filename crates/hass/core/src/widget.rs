//! What the pill is showing, and what a scroll means while it shows that.
//!
//! The whole point of this type is that a scroll notch is ambiguous: the same
//! event brightens the lamp or warms it depending on which of the two the pill
//! is currently displaying, and which one that is belongs to the *widget*, not
//! to the lamp. Home Assistant has no opinion on it and nothing to store it in.
//!
//! Pure, and the reason [`crate::lamp`] and this are separate modules: every
//! test below runs without a Home Assistant to talk to.
//!
//! [`Snapshot`] is the JSON the bar reads, and its shape is a contract with the
//! yuck rather than an internal detail — `state`, `type` and `value` are the
//! three fields `hass-entry.yuck` branches on. Changing a name here means
//! changing it there in the same commit.

use serde::Serialize;

use crate::lamp::{Change, LampState, Status};
use crate::proto::Command;

/// Percentage points per scroll notch.
const STEP: f32 = 10.0;

/// One line of JSON for the bar: `{"state":"on","type":"brightness","value":42}`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Snapshot {
    On(Reading),
    Off,
}

/// What an on lamp is showing. Adjacently tagged, so it flattens into the
/// object above rather than nesting.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum Reading {
    Brightness(f32),
    Temperature(f32),
    /// On, but with no number to show yet.
    Loading,
}

/// Which of the two an on pill is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Brightness,
    Temperature,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    On(Mode),
    Off,
}

#[derive(Debug, Clone, Copy)]
pub struct Widget {
    state: State,
}

impl Widget {
    /// Start from what the lamp is doing, showing brightness if it is on.
    pub fn new(lamp: &LampState) -> Self {
        Self {
            state: match lamp.status {
                Status::On => State::On(Mode::Brightness),
                Status::Off => State::Off,
            },
        }
    }

    /// Fold a command in, returning the change the lamp should be asked for.
    ///
    /// `None` covers both "nothing to do" and "that was about the widget": a
    /// scroll on an off pill is ignored, and toggling between brightness and
    /// temperature never touches the lamp.
    pub fn apply(&mut self, command: Command) -> Option<Change> {
        match command {
            Command::Increase | Command::Decrease => {
                let State::On(mode) = self.state else {
                    return None;
                };
                let delta = match command {
                    Command::Increase => STEP,
                    _ => -STEP,
                };
                Some(match mode {
                    Mode::Brightness => Change::DeltaBrightness(delta),
                    Mode::Temperature => Change::DeltaTemperature(delta),
                })
            }
            Command::ToggleMode => {
                if let State::On(mode) = &mut self.state {
                    *mode = match mode {
                        Mode::Brightness => Mode::Temperature,
                        Mode::Temperature => Mode::Brightness,
                    };
                }
                None
            }
            Command::ToggleState => Some(match self.state {
                State::On(_) => {
                    self.state = State::Off;
                    Change::TurnOff
                }
                State::Off => {
                    self.state = State::On(Mode::Brightness);
                    Change::TurnOn
                }
            }),
        }
    }

    /// The line to publish, for this widget over that lamp.
    ///
    /// A missing value is [`Reading::Loading`] rather than a zero, because zero
    /// is a brightness and "not known yet" is not.
    pub fn snapshot(&self, lamp: &LampState) -> Snapshot {
        match self.state {
            State::Off => Snapshot::Off,
            State::On(mode) => Snapshot::On(
                match mode {
                    Mode::Brightness => lamp.brightness.map(Reading::Brightness),
                    Mode::Temperature => lamp.temperature.map(Reading::Temperature),
                }
                .unwrap_or(Reading::Loading),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lamp::Range;

    fn lamp_on() -> LampState {
        LampState {
            status: Status::On,
            brightness: Some(50.0),
            temperature: Some(50.0),
            temperature_range: Some(Range::new(153.0, 500.0)),
        }
    }

    fn lamp_off() -> LampState {
        LampState {
            status: Status::Off,
            brightness: None,
            temperature: None,
            temperature_range: Some(Range::new(153.0, 500.0)),
        }
    }

    /// The sequence the old binary's one test walked, kept: a scroll moves
    /// brightness, a mode toggle moves nothing, the next scroll moves
    /// temperature, and clicks toggle the lamp.
    #[test]
    fn a_scroll_means_whichever_mode_the_pill_is_in() {
        let mut widget = Widget::new(&lamp_on());

        assert_eq!(
            widget.apply(Command::Increase),
            Some(Change::DeltaBrightness(STEP))
        );
        assert_eq!(widget.apply(Command::ToggleMode), None);
        assert_eq!(
            widget.apply(Command::Increase),
            Some(Change::DeltaTemperature(STEP))
        );
        assert_eq!(widget.apply(Command::ToggleState), Some(Change::TurnOff));
        assert_eq!(widget.apply(Command::ToggleState), Some(Change::TurnOn));
    }

    #[test]
    fn a_scroll_down_is_the_step_the_other_way() {
        let mut widget = Widget::new(&lamp_on());
        assert_eq!(
            widget.apply(Command::Decrease),
            Some(Change::DeltaBrightness(-STEP))
        );
    }

    #[test]
    fn scrolling_an_off_pill_does_nothing_at_all() {
        let mut widget = Widget::new(&lamp_off());
        assert_eq!(widget.apply(Command::Increase), None);
        assert_eq!(widget.apply(Command::ToggleMode), None);
        // And it is still off afterwards, rather than having been switched by a
        // scroll nobody meant as a click.
        assert_eq!(widget.snapshot(&lamp_off()), Snapshot::Off);
    }

    #[test]
    fn turning_it_on_shows_brightness_first() {
        let mut widget = Widget::new(&lamp_off());
        widget.apply(Command::ToggleState);
        assert_eq!(
            widget.snapshot(&lamp_on()),
            Snapshot::On(Reading::Brightness(50.0))
        );
    }

    #[test]
    fn the_json_is_the_three_fields_the_yuck_reads() {
        let json = |snapshot: Snapshot| serde_json::to_string(&snapshot).unwrap();

        assert_eq!(json(Snapshot::Off), r#"{"state":"off"}"#);
        assert_eq!(
            json(Snapshot::On(Reading::Brightness(42.0))),
            r#"{"state":"on","type":"brightness","value":42.0}"#
        );
        assert_eq!(
            json(Snapshot::On(Reading::Temperature(7.5))),
            r#"{"state":"on","type":"temperature","value":7.5}"#
        );
        assert_eq!(
            json(Snapshot::On(Reading::Loading)),
            r#"{"state":"on","type":"loading"}"#
        );
    }

    #[test]
    fn a_lamp_still_coming_up_reads_as_loading_not_as_zero() {
        let settling = LampState {
            brightness: None,
            ..lamp_on()
        };
        let widget = Widget::new(&settling);
        assert_eq!(widget.snapshot(&settling), Snapshot::On(Reading::Loading));
    }
}
