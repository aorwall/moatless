// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Tasks band (band 5): task lifecycle, messages, agent turns, and the feeds
//! over them.
//!
//! CRUD, close/reopen and reap; tool responses; activities; event ingress;
//! feedback; task events; tags; the Task listing feed; and file storage.
//!
//! The backend re-exports these paths through permanent facade modules under
//! `backend/src/`, so higher bands keep naming `crate::task::…` while the
//! backend stays the composition root; every router here is generic over state
//! `S` via `FromRef<S>`, so this crate names no concrete `AppState`.

pub mod activities;
pub mod agent;
pub mod agent_event;
pub mod artifact;
pub mod collection;
pub mod compose;
pub mod conversation;
pub mod credential_sync;
pub mod event_ingress;
pub mod feedback;
pub mod inbox;
pub mod memory;
pub mod message;
pub mod ports;
pub mod review_guide;
pub mod scope_reach;
pub mod search;
pub mod secret_request;
pub mod tag;
pub mod task;
pub mod task_event;
pub mod task_listing;
pub mod task_turn;
pub mod trajectory;
