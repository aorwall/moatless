//! Linear webhook ingress and outcome vocabulary.

pub mod reason;
mod receive;

pub use receive::{Outcome, IngressError, Rejection, receive};
pub(crate) use receive::{is_issue_state_change, issue_state_transition_id};
