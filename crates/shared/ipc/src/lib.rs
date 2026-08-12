//! The wire protocol between a daemon and its clients.
//!
//! Messages are length-prefixed bincode: a 4-byte little-endian length, then
//! that many bytes of a bincode-serialized request or response. Every daemon in
//! this workspace speaks it, so the framing lives here and only the message
//! *types* stay per-app — those are the part that differs, and the part where a
//! shared enum would be a liability rather than a saving.
//!
//! Most connections carry one request and one response, then close: call volume
//! is tiny (a click, a shell command), so the cheap unix-socket connect is fine
//! and the framing stays trivial. A subscription is the one exception — it
//! holds the connection open and the daemon writes a frame down it on every
//! change, so the bar's countdown moves without the bar owning a clock.
//!
//! Two knobs are deliberately per-app rather than constants here, because their
//! right values come from what the app actually sends:
//!
//! * [`Endpoint::max_frame`] — a corrupt or hostile length header must not talk
//!   the reader into a huge allocation before a single body byte has arrived.
//!   timers sends a few hundred bytes per timer; a claude-usage month chart is
//!   a few kilobytes. Both want a ceiling far above anything real and far below
//!   anything ruinous, and those are different numbers.
//! * [`Endpoint::watchdog`] — how long a subscriber waits before deciding the
//!   daemon is wedged and reconnecting. timers publishes on a 30 s heartbeat
//!   precisely so silence stays meaningful, so three heartbeats is generous;
//!   claude-usage only speaks when a poll lands, minutes apart, so the same
//!   number there would be a reconnect loop.
//!
//! One caveat that applies to every message type carried over this: bincode
//! encodes enum variants by index, so adding a variant is safe only as long as
//! the daemon and the binaries talking to it are deployed together. A client
//! from an older build reads a new variant as garbage. In this workspace they
//! always are deployed together — one flake, one closure — which is part of why
//! the framing can stay this simple.

use std::io::{Read, Write};
use std::marker::PhantomData;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The width of the length prefix, in bytes.
pub const LEN_HEADER: usize = 4;

/// How long a one-shot exchange waits on the socket.
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Encode a message as length-prefixed bincode.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let body = bincode::serialize(value)?;
    let len = body.len() as u32;
    let mut out = len.to_le_bytes().to_vec();
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode a length-prefixed bincode message from a full buffer — the caller
/// must have already read the length and then that many body bytes.
pub fn decode<T: DeserializeOwned>(buf: &[u8]) -> Result<T> {
    Ok(bincode::deserialize(buf)?)
}

/// Check a length header before allocating a buffer for it.
pub fn check_frame_len(len: u32, max: u32) -> Result<usize> {
    if len > max {
        anyhow::bail!("frame of {len} bytes exceeds the {max}-byte limit");
    }
    Ok(len as usize)
}

/// Read one length-prefixed frame off a stream.
///
/// A clean EOF at the header boundary means the peer closed without replying;
/// anything shorter is a protocol error. Both arrive as an `Err` here, and both
/// mean the same thing to a caller: stop using this connection.
pub fn read_frame<T: DeserializeOwned>(stream: &mut impl Read, max_frame: u32) -> Result<T> {
    let mut header = [0u8; LEN_HEADER];
    stream.read_exact(&mut header)?;
    let len = check_frame_len(u32::from_le_bytes(header), max_frame)?;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    decode(&body)
}

/// Write one length-prefixed frame, flushed.
pub fn write_frame<T: Serialize>(stream: &mut impl Write, value: &T) -> Result<()> {
    stream.write_all(&encode(value)?)?;
    stream.flush()?;
    Ok(())
}

/// Bind a daemon's control socket, creating its directory and clearing a stale
/// one first.
///
/// The unlink is not optional: a socket left behind by a killed daemon makes
/// `bind` fail with EADDRINUSE even though nothing is listening, and the daemon
/// then refuses to start until someone removes a file by hand.
///
/// `mode` is per-app because the audience is. A per-user daemon wants 0600 —
/// nothing else on the machine has any business adding timers to this session.
/// claude-usage wants 0660, because it runs as its own user and the bar reaches
/// it through a shared group.
pub fn bind(socket: &Path, mode: u32) -> Result<UnixListener> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let _ = std::fs::remove_file(socket);
    let listener =
        UnixListener::bind(socket).with_context(|| format!("binding {}", socket.display()))?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(mode))?;
    Ok(listener)
}

/// A daemon's socket, from a client's side.
///
/// Holds the address and the two per-app knobs, so a caller passes them once at
/// construction rather than at every request.
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub socket: PathBuf,
    pub max_frame: u32,
    pub watchdog: Duration,
}

impl Endpoint {
    pub fn new(socket: PathBuf, max_frame: u32, watchdog: Duration) -> Self {
        Self {
            socket,
            max_frame,
            watchdog,
        }
    }

    /// Connect, send a request, read the response, close.
    ///
    /// Returns an error if the socket is missing, the connect fails, or the
    /// daemon sends back malformed bytes. Callers should treat any error as
    /// "daemon unavailable" and fall back to a safe default rather than retry:
    /// the bar has a line to print either way, and a retry loop here would only
    /// move the stall somewhere less visible.
    pub fn request<Q: Serialize, R: DeserializeOwned>(&self, req: &Q) -> Result<R> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
        write_frame(&mut stream, req)?;
        read_frame(&mut stream, self.max_frame)
    }

    /// Connect and subscribe. The daemon answers immediately with its current
    /// snapshot, so the first [`Subscription::recv`] returns without waiting.
    pub fn subscribe<Q: Serialize, R: DeserializeOwned>(&self, req: &Q) -> Result<Subscription<R>> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
        stream.set_read_timeout(Some(self.watchdog))?;
        write_frame(&mut stream, req)?;
        Ok(Subscription {
            stream,
            max_frame: self.max_frame,
            _response: PhantomData,
        })
    }
}

/// A held-open subscription.
///
/// An error from [`recv`](Self::recv) ends it — the daemon restarted, or the
/// watchdog fired — and the caller should drop it and open a fresh one rather
/// than call again.
pub struct Subscription<R> {
    stream: UnixStream,
    max_frame: u32,
    _response: PhantomData<fn() -> R>,
}

impl<R: DeserializeOwned> Subscription<R> {
    /// Block until the daemon publishes the next frame.
    pub fn recv(&mut self) -> Result<R> {
        read_frame(&mut self.stream, self.max_frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips() {
        let bytes = encode(&("hola", 42u32)).unwrap();
        assert_eq!(
            bytes.len() - LEN_HEADER,
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize
        );

        let mut cursor = std::io::Cursor::new(bytes);
        let back: (String, u32) = read_frame(&mut cursor, 1024).unwrap();
        assert_eq!(back, ("hola".to_string(), 42));
    }

    #[test]
    fn an_oversized_header_is_refused_before_allocating() {
        assert!(check_frame_len(u32::MAX, 1024).is_err());
        assert_eq!(check_frame_len(1024, 1024).unwrap(), 1024);
    }
}
