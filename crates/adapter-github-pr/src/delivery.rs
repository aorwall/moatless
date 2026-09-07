//! Durable GitHub PR delivery vocabulary and registration.

pub(crate) mod kind;
mod registration;

pub use registration::register;
pub(crate) use registration::{ADAPTER, DeliveryContext, PrMessageDelivery, CloseDelivery, PrMergedDelivery, PostMergeDelivery, MergeConflictDelivery, request, log_admission};
