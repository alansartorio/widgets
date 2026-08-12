//! What the timers daemon and its clients say to each other.
//!
//! The framing — length-prefixed bincode — lives in [`widgets_ipc`] and is
//! shared with claude-usage. What is left here is the vocabulary, which is not:
//! only two of the six requests below would mean anything to another daemon.
//!
//! Most connections carry one request and one response, then close — call
//! volume is tiny (a click, a shell command), so the cheap unix-socket connect
//! is fine. [`Request::Subscribe`] is the one exception: it holds the
//! connection open and the daemon writes a [`Response::Status`] frame down it
//! on every change and on every tick, so the bar's countdown moves without the
//! bar owning a clock.

use serde::{Deserialize, Serialize};

/// Largest frame either side will allocate for. A status frame is a few hundred
/// bytes per timer, so this is far above anything real; it exists so a corrupt
/// or hostile length header can't talk the reader into a huge allocation before
/// a single body byte has arrived.
pub const MAX_FRAME: u32 = 1024 * 1024;

/// How long a subscriber waits before deciding the daemon is wedged.
///
/// A subscriber hears from the daemon every second while a timer is counting
/// down, and every keepalive interval when nothing is — the daemon publishes on
/// a 30 s heartbeat precisely so that silence stays meaningful. Three
/// heartbeats of nothing means reconnecting beats waiting on it forever.
pub const SUBSCRIBE_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(90);

/// The socket mode. 0600: this daemon is per-user and so is everything it
/// holds. Nothing else on the machine has any business adding timers to this
/// session.
pub const SOCKET_MODE: u32 = 0o600;

use crate::models::Status;

/// A client's ask of the daemon.
///
/// Both sides are built from this crate, so adding a variant is safe as long as
/// the daemon and the binaries talking to it are deployed together — bincode
/// encodes enum variants by index, and a client from an older build would read a
/// new variant as garbage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    /// The current snapshot. One frame, then close.
    List,
    /// Create a timer. `deadline` is absolute unix seconds: the client parsed
    /// the timespan (both clients share `timespec`, so they agree), and the
    /// daemon only stores what it is told.
    ///
    /// `private` keeps the name off the bar. It is settable only here, at
    /// creation — there is no request to change it afterwards.
    Add {
        name: String,
        deadline: i64,
        private: bool,
    },
    /// Delete a timer whether or not it has fired. An unknown id is a no-op,
    /// not an error — two clients racing to remove the same row should both
    /// succeed.
    Remove { id: i64 },
    /// Acknowledge a timer that has fired, which deletes it. Acknowledging one
    /// that is still counting down is an error; that is what `Remove` is for.
    Ack { id: i64 },
    /// Acknowledge everything that has fired — the bar's middle click.
    AckAll,
    /// Stream the snapshot instead of asking for it. The daemon answers with
    /// the current one straight away and then writes another frame on every
    /// change and every tick, until the client goes away.
    Subscribe,
}

/// The daemon's reply.
///
/// Every mutation answers with the resulting [`Status`] rather than an ack, so
/// the CLI prints one shape whatever the subcommand and a caller never has to
/// follow a write with a read.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Response {
    Status(Status),
    Error(String),
}
