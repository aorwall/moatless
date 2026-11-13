//! Agent process and session lifetime, input delivery, and normalized signals.
//!
//! A [`HarnessRuntime`] controls at most one active process while its session survives
//! process replacement. The coordinator owns Run/Turn identity,
//! acknowledgement, retry policy, credentials, and backend publishing.

pub mod content;
pub mod error;
pub mod signal;
pub mod source;
pub mod trajectory;
mod transcode;

mod claude;
pub(crate) mod codex;
mod driver;
mod mini_swe;
mod pi;
pub use mini_swe::MiniSweConfig;
mod process;

// The scripted mock and its `HarnessRuntime::scripted` constructor exist only to drive
// the lifecycle/contract tests, so they stay behind `cfg(test)` — never part of
// the crate's runtime surface.
#[cfg(test)]
mod scripted;

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod persistent_tests;

pub use claude::ClaudeConfig;
pub(crate) use claude::continued_transcript as claude_continued_transcript;
pub use codex::CodexConfig;
pub use error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
pub use pi::PiConfig;
#[cfg(test)]
pub(crate) use scripted::{Script, Step};
pub use signal::{
    Activity, Cause, ContextWindow, Input, InputId, Outcome, Rejection, RunTotals, SessionId,
    Signal,
};
pub use source::Role;

mod runtime;
pub use runtime::{HarnessClosed, HarnessRuntime};
