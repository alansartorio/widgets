//! A sync client for the timers daemon's socket.
//!
//! Two shapes of conversation, both of them [`widgets_ipc`]'s: [`request`] is
//! one request per connection — connect, send, read, close — used by the GUI
//! and by every CLI subcommand but one. [`subscribe`] is the long-lived one:
//! subscribe once, then read a frame every time the daemon publishes, which is
//! what `timers tail` feeds the bar from.
//!
//! What this module adds over calling `widgets_ipc` directly is the three
//! constants that make an endpoint a *timers* endpoint — the socket path, the
//! frame ceiling and the watchdog — so no caller has to remember them.

use widgets_ipc::Endpoint;

use crate::paths::Paths;
use crate::proto::{MAX_FRAME, Request, Response, SUBSCRIBE_WATCHDOG};

/// A held-open snapshot subscription.
///
/// An error from `recv` ends it — the daemon restarted, or the watchdog fired —
/// and the caller should drop it and open a fresh one rather than call again.
pub type Subscription = widgets_ipc::Subscription<Response>;

fn endpoint(paths: &Paths) -> Endpoint {
    Endpoint::new(paths.socket(), MAX_FRAME, SUBSCRIBE_WATCHDOG)
}

/// Connect to the daemon, send a request, read the response.
///
/// Returns an error if the socket is missing, the connect fails, or the daemon
/// sends back malformed bytes. Callers should treat any error as "daemon
/// unavailable" and fall back to a safe default rather than retry.
pub fn request(paths: &Paths, req: &Request) -> anyhow::Result<Response> {
    endpoint(paths).request(req)
}

/// Connect and subscribe. The daemon answers immediately with its current
/// snapshot, so the first `recv` returns without waiting.
pub fn subscribe(paths: &Paths) -> anyhow::Result<Subscription> {
    endpoint(paths).subscribe(&Request::Subscribe)
}
