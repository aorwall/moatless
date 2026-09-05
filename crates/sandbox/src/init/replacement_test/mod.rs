//! Sandbox-replacement lifecycle tests.
//!
//! When a pod is replaced, `run_init` restores Claude/Codex Session state to
//! disk (before Ready), decides interrupted-Run vs pending-Message precedence
//! while external dispatch is still gated, then opens the gate and performs the
//! one authoritative dispatch (`init::open_for_work`).
//!
//! The tests observe the **harness** end of the path, not the command channel:
//! each lifecycle action drives a real [`CoordinatorService`] over a recording
//! scripted [`Harness`], and assertions read back the exact `Input` the harness
//! accepted (text + resume) — proving harness acceptance, not enqueue shape.
//!
//! Split into cohesive submodules:
//! - [`backend`] — the in-memory `ReplayBackend`.
//! - [`support`] — recording-harness state and small fixtures.
//! - [`readiness`] — the readiness gate and the startup precedence race.
//! - [`precedence`] — interrupted-Run vs pending-Message precedence + fail-closed.
//! - [`restore`] — session-state archive restore (happy / missing / corrupt) and
//!   the restore-then-replay lifecycle.
//! - [`lost_commands`] — the wake for `moat cmd` commands the replacement
//!   destroyed, which no interrupted run covers because the turn had completed.
//!
//! [`CoordinatorService`]: crate::agent::coordinator::CoordinatorService
//! [`Harness`]: crate::agent::harness::Harness

mod backend;
mod support;

mod lost_commands;
mod precedence;
mod readiness;
mod restore;
