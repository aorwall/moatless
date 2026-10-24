//! Linear webhook ingress and outcome vocabulary.

pub mod reason;
mod receive;

pub use receive::{IngressError, Outcome, Rejection, receive};
pub(crate) use receive::{
    added_label_ids, issue_state_transition_id, new_state_id, previous_state_id,
};
