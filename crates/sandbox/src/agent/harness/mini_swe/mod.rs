//! mini-swe-agent process transport and native message journal.
mod config;
mod process;
mod session;
pub use config::MiniSweConfig;
pub(crate) use process::MiniSweProcess;
