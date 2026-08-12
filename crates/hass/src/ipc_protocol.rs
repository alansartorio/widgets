use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Message {
    Increase,
    Decrease,
    ToggleMode,
    ToggleState,
}
