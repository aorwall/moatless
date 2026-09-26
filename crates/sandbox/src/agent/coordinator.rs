//! The harness-neutral coordinator.
//!
//! [`seam`] maps one [`HarnessRuntime`](super::harness::HarnessRuntime)'s signal stream onto
//! Run events and acknowledgements. [`service`] drives that mapping from the
//! [`AgentCommand`](super::handle::AgentCommand) channel and owns the lifecycle
//! policy around it.

mod seam;
mod service;

pub(crate) use seam::Coordinator;
pub(crate) use service::CoordinatorService;

#[cfg(test)]
mod tests;
