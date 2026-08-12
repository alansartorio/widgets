//! What the bar sends: one word per message.
//!
//! Not the workspace's bincode framing, and that is on purpose. The peer is a
//! yuck string running `socat`, not a binary from this flake:
//!
//! ```yuck
//! :onscroll "socat -u - UNIX-SENDTO:/tmp/hass-light.sock <<< increase"
//! ```
//!
//! A length prefix would mean a shell computing one, so the wire format stays a
//! bare word.
//!
//! Parsed by hand rather than through `serde_plain`, which is the last
//! dependency this crate had that nothing else in the workspace uses: four
//! variants against a whole crate, and this way an unknown word can name itself
//! in the error.

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// One step up, on whatever the pill is currently showing.
    Increase,
    /// One step down, likewise.
    Decrease,
    /// Switch the pill between brightness and colour temperature.
    ToggleMode,
    /// Turn the lamp on or off.
    ToggleState,
}

impl Command {
    /// Parse one message off the wire.
    ///
    /// Whitespace-tolerant because `<<<` adds a newline and nobody writing that
    /// yuck should have to know it.
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(match raw.trim() {
            "increase" => Self::Increase,
            "decrease" => Self::Decrease,
            "toggle_mode" => Self::ToggleMode,
            "toggle_state" => Self::ToggleState,
            other => bail!(
                "unknown command {other:?}; expected increase, decrease, toggle_mode or toggle_state"
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_words_the_yuck_sends() {
        assert_eq!(Command::parse("increase").unwrap(), Command::Increase);
        assert_eq!(Command::parse("decrease").unwrap(), Command::Decrease);
        assert_eq!(Command::parse("toggle_mode").unwrap(), Command::ToggleMode);
        assert_eq!(
            Command::parse("toggle_state").unwrap(),
            Command::ToggleState
        );
    }

    #[test]
    fn a_trailing_newline_is_not_a_different_command() {
        assert_eq!(Command::parse("increase\n").unwrap(), Command::Increase);
    }

    #[test]
    fn an_unknown_word_names_itself() {
        let err = Command::parse("brighter").unwrap_err().to_string();
        assert!(err.contains("brighter"), "{err}");
    }
}
