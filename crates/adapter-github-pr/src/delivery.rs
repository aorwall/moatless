//! Durable GitHub PR delivery vocabulary and registration.

pub(crate) mod kind;
mod registration;

pub use registration::register;
pub(crate) use registration::{
    ADAPTER, CheckRunDelivery, CheckRunTriggerDelivery, CloseDelivery, DeliveryContext,
    MergeConflictDelivery, PostMergeDelivery, PrMergedDelivery, PrMessageDelivery, log_admission,
    request,
};
