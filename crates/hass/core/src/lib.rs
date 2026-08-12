//! The bar's light pill: Home Assistant's REST API, one lamp, and the little
//! state machine that turns scrolls and clicks into service calls.
//!
//! The pieces, in the order a scroll event moves through them:
//!
//! ```text
//! proto::Command     one word off the bar: increase, toggle_mode, …
//! widget::Widget     what the pill is showing, and what that word means there
//! lamp::Lamp         the same change, as a Home Assistant service call
//! api::Client        the HTTP
//! widget::Snapshot   the JSON line the bar reads back
//! ```
//!
//! The split from the daemon is the point of this crate: everything here is
//! testable without a Home Assistant to talk to, and everything that owns a
//! socket lives next door.
//!
//! Module names are this workspace's rather than the ones it arrived with —
//! `api`, `proto`, `widget`, as in timers and claude-usage. `lamp_simulator`
//! folded into `lamp`: it was the lamp's state and two methods on it, and the
//! name promised a simulation that was never there.

pub mod api;
pub mod lamp;
pub mod proto;
pub mod widget;
