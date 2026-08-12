//! Owns stdout and the last reading written to it.
//!
//! The tick rate and the rate at which anything changes are two different
//! things: a running entry really does read differently every second, but an
//! idle one only moves when the poll lands a minute later, and a failure
//! repeats its one error string for as long as the server stays away. Sending
//! those unchanged is a variable update and a widget re-render sixty times a
//! minute for nothing.
//!
//! Suppressing them is safe because eww's `deflisten` keeps the last value it
//! was given — there is no keepalive to maintain, and nothing to re-send after
//! a quiet stretch.

use std::io::{self, Write};
use std::path::Path;

use crate::output::Output;

pub struct Emitter {
    last: Option<Output>,
    /// Where to drop a copy of the last line, for the GUI to open on. `None`
    /// for a one-shot invocation, which has nobody to hand it to.
    mirror: Option<std::path::PathBuf>,
}

impl Emitter {
    pub fn new() -> Self {
        Self {
            last: None,
            mirror: None,
        }
    }

    /// Also write every emitted line to `path`.
    ///
    /// This is a cache, not a protocol. `kimai-gui` reads it on open so its
    /// window has numbers in it before its own first poll returns, and treats a
    /// missing or stale file as "cargando…". Nothing depends on it being there.
    pub fn mirroring(mut self, path: impl AsRef<Path>) -> Self {
        self.mirror = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn send(&mut self, output: Output) {
        if self.last.as_ref() == Some(&output) {
            return;
        }

        let line = serde_json::to_string(&output).expect("output is always serialisable");

        // Rust's stdout is line buffered, so each line reaches the listening bar
        // as it is written rather than waiting for a full buffer.
        //
        // The bar closing the pipe — a reload, an eww restart — is this
        // process's job ending, not a crash to report. `println!` would panic
        // and leave a backtrace in eww's log for something entirely expected.
        if writeln!(io::stdout(), "{line}").is_err() {
            std::process::exit(0);
        }

        self.mirror(&line);
        self.last = Some(output);
    }

    /// Best-effort, and silent when it fails: a read-only runtime directory
    /// should cost the GUI a fast first paint, not cost the bar its widget.
    fn mirror(&self, line: &str) {
        let Some(path) = &self.mirror else { return };
        let Some(parent) = path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        // Temp-and-rename so the GUI cannot read a half-written line.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, line).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    pub fn failure(&mut self, error: &str) {
        self.send(Output::failed(error));
    }
}

impl Default for Emitter {
    fn default() -> Self {
        Self::new()
    }
}

/// Flatten an error chain into the one line the widget has to say it in.
///
/// "cannot read the timesheet list: expected value at line 1" says more than
/// either half alone.
pub fn describe(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(|cause| cause.to_string())
        .collect::<Vec<_>>()
        .join(": ")
}
