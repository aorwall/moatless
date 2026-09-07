//! Compatibility exports for the owning `outbox_dao` persistence module.

pub use super::outbox_dao::{
    ClaimedPayload, IngressOutbox, IngressPayload, Pending, PgIngressOutbox,
};
pub(super) use super::outbox_dao::{RETRY_BASE_SECS, RETRY_CAP_SECS};
