//! Linear webhook ingress and outcome vocabulary.

pub mod reason;
mod receive;

pub use receive::{Outcome, IngressError, Rejection, receive};
pub(crate) use receive::issue_state_transition_id;
