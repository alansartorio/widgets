//! Shared library for the claude-usage tracker: data models, the poll API,
//! the SQLite store, and the bincode socket protocol used by the daemon's
//! clients (the CLI and the GUI).
//!
//! The daemon is the only process that touches the database or the token; the
//! CLI and GUI talk to it over a unix socket using the [`proto`] messages.

pub mod api;
pub mod client;
pub mod db;
pub mod models;
pub mod paths;
pub mod proto;
