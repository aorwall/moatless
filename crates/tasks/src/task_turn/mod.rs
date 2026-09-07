//! Task turn state module.
//!
//! Authoritative per-turn lifecycle state (`task_turns` table), written
//! transactionally at the gRPC event ingress so events become projections of
//! this state rather than its source of truth.

pub mod dao;
pub mod models;

pub mod recovery;
