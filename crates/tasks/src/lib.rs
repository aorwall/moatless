// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Tasks band (band 5): task lifecycle (CRUD, close/reopen, reap), messages and
//! tool responses, agent turns and activities, event ingress, feedback, task
//! events, tags, the Task listing feed, and file storage.
//!
//! The backend composes this crate through permanent facade modules
//! (`backend/src/{task,message,agent,agent_event,credential_sync,event_ingress,
//! feedback,tag,task_activities,task_event,task_listing,task_turn}`) that re-export
//! these paths, so the higher bands still in the backend keep naming
//! `crate::task::…` etc. The facade is not transitional: the backend stays the
//! composition root that wires these ports into `AppState`. Every router here is
//! generic over the state `S` with `FromRef<S>` bounds, so this crate names no
//! concrete `AppState` — the `FromRef` impls live in the backend.

pub mod agent;
pub mod agent_event;
pub mod compose;
pub mod credential_sync;
pub mod event_ingress;
pub mod feedback;
pub mod message;
pub mod ports;
pub mod tag;
pub mod task;
pub mod task_activities;
pub mod task_event;
pub mod task_listing;
pub mod task_turn;
