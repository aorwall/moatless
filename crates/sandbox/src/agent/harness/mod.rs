//! Agent process and session lifetime, input delivery, and normalized signals.
//!
//! A [`Harness`] controls at most one active process while its session survives
//! process replacement. The coordinator owns Run/Turn identity,
//! acknowledgement, retry policy, credentials, and backend publishing.

pub mod content;
pub mod error;
pub mod signal;
pub mod source;

mod claude;
pub(crate) mod codex;
mod driver;
mod process;

// The scripted mock and its `Harness::scripted` constructor exist only to drive
// the lifecycle/contract tests, so they stay behind `cfg(test)` — never part of
// the crate's runtime surface.
#[cfg(test)]
mod scripted;

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod persistent_tests;

pub use claude::ClaudeConfig;
pub use codex::CodexConfig;
pub use error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
#[cfg(test)]
pub(crate) use scripted::{Script, Step};
pub use signal::{
    Activity, Cause, ContextWindow, Input, InputId, Outcome, Rejection, SessionId, Signal,
};
pub use source::{NativeMessageId, Role, SourceEvent};

mod runtime;
pub use runtime::{Harness, HarnessClosed};
