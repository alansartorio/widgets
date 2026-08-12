//! What the claude-usage daemon and its clients say to each other.
//!
//! The framing — length-prefixed bincode — lives in [`widgets_ipc`] and is
//! shared with timers. What is left here is the vocabulary, which is not.
//!
//! Most connections carry one request and one response, then close — call
//! volume is tiny (the GUI per click), so the cheap unix-socket connect is
//! fine. [`Request::Subscribe`] is the one exception: it holds the connection
//! open and the daemon writes a [`Response::Status`] frame down it every time a
//! poll lands, so the bar never has to ask.

/// Largest frame either side will allocate for. A month chart is a few
/// kilobytes, so this is far above anything real; it exists so a corrupt or
/// hostile length header can't talk the reader into a multi-gigabyte
/// allocation before a single body byte has arrived.
pub const MAX_FRAME: u32 = 8 * 1024 * 1024;

/// How long a subscriber waits before deciding the daemon is wedged.
///
/// A subscriber only hears from the daemon when a poll lands, which is minutes
/// apart by design. This is not a per-frame deadline, then, but a watchdog: a
/// daemon that has gone quiet for well past two poll intervals is stuck, and
/// reconnecting is better than waiting on it forever.
pub const SUBSCRIBE_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// The socket mode. 0660 rather than timers' 0600: this daemon runs as its own
/// user, and the bar — running as the desktop user — reaches it through the shared
/// `claude-usage` group.
pub const SOCKET_MODE: u32 = 0o660;

use serde::{Deserialize, Serialize};

use crate::models::{Chart, UsageStatus, View};

/// A client's ask of the daemon.
///
/// Both sides are built from this crate, so adding a variant is safe as long
/// as the daemon and the binaries talking to it are deployed together — bincode
/// encodes enum variants by index, and a client from an older build would read
/// a new variant as garbage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    /// The pill's data: the last poll's status. One frame, then close.
    Status,
    /// The chart for a day or the last 30 days. One frame, then close.
    Chart { view: View },
    /// Stream the status instead of asking for it. The daemon answers with the
    /// current snapshot straight away and then writes another frame each time
    /// a poll lands, until the client goes away.
    Subscribe,
    /// Poll the endpoint now rather than waiting out the interval — what the
    /// bar's "retry" click asks for. Answers with the status the poll
    /// produced, and every subscriber sees the same update pushed to them.
    PollNow,
}

/// The daemon's reply.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Response {
    Status(UsageStatus),
    Chart(Chart),
    Error(String),
}

