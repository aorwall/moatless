//! Generic root-supervision primitives for a process's long-lived tokio tasks.
//!
//! A process registers each [`tokio::task::JoinHandle`] here instead of
//! discarding it, so an early return or a panic becomes a [`TaskExit`] rather
//! than vanishing; what to do about that is the process's own [`Policy`].
//!
//! This crate owns only what does not vary with policy: the exit shape
//! ([`ExitCause`], [`Completion`]), the ledger a probe reads
//! ([`SupervisorHealth`]), and the join-set accounting ([`watch`],
//! [`classify`], [`drain`]). Registration stays with each process.

mod exit;
mod health;
mod join;
mod policy;

pub use exit::{Completion, ExitCause, TaskExit};
pub use health::SupervisorHealth;
pub use join::{Joined, Supervised, classify, drain, watch};
pub use policy::{Outcome, Policy};
