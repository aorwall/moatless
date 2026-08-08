//! Log-read options for the sandbox status seams.
//!
//! The sandbox runtime ports — `SandboxObserver`, `SandboxIntentReader`,
//! `SandboxStarter`, `SandboxStopper`, `SandboxConnector` — are declared once,
//! in `moatless_backend_core::sandbox`, and both this crate and `t3code` depend
//! on that one declaration. The backend's full-fidelity HTTP read lives in
//! `super::status_reader::SandboxStatusReader`. This module keeps only the
//! log-read options both share.

/// Which slice of a server container's log to read.
#[derive(Debug, Clone, Copy)]
pub struct LogOptions {
    /// Read the previous (crashed/restarted) container instance instead of the
    /// live one. Implies no follow — that instance is gone.
    pub previous: bool,
    /// Hold the stream open and append lines as the container writes them.
    /// `false` returns what is already buffered and ends the stream.
    pub follow: bool,
    /// How many lines back from the end to start.
    pub tail_lines: i64,
}

impl Default for LogOptions {
    fn default() -> Self {
        Self {
            previous: false,
            follow: true,
            tail_lines: 500,
        }
    }
}
