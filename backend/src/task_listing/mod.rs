//! The durable record that a Task's or Workspace's listing-visible state changed.
//!
//! The record is written by the code performing the mutation, in that
//! mutation's own transaction, and its position is allocated there too — from
//! one counter row for the whole deployment, held until commit. A `bigserial`
//! will not do: an id assigned at INSERT and made visible at COMMIT can be read
//! out of order, so two writers take 100 and 101, 101 commits first, and a
//! reader polling `> 99` advances past 100 and never sees it, silently, while
//! reporting itself up to date. Holding the counter makes commit order and
//! position order the same order; what that costs callers is
//! [`dao::record_change`]'s "call this last".
//!
//! Decisions 2 and 7 of `specs/t3-shell-projection.md` carry the argument. No T3
//! vocabulary appears here — [`crate::ui_rpc`] turns the record into a shell
//! event.

pub mod dao;
pub mod models;
pub mod pruner;

pub use dao::{
    changes_after, current_head, current_position, prune_changes, record_change,
    record_workspace_change,
};
pub use models::{
    resume, ChangeSubject, EnvironmentIdentity, ListingChange, ListingHead, Resume, RESUME_WINDOW,
    RETAINED_CHANGES,
};
pub use pruner::spawn_change_pruner;
