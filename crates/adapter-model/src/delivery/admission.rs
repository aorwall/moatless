//! The one seam a provider crosses to hand a delivery over durably.
//!
//! A webhook handler holds a [`DeliveryAdmission`], never the store behind it:
//! the store's other operations belong to the worker that owns admitted
//! records, and a handler that could reach them is a handler that could claim
//! or finish work it has no business touching. The failure type names no
//! database driver, so a crate that admits deliveries compiles none.

use async_trait::async_trait;

use super::{AdapterDeliveryRequest, Admission, AdmitError};

/// Take durable ownership of one inbound Adapter delivery.
///
/// One method: no Adapter crate calls anything else on the store behind it.
#[async_trait]
pub trait DeliveryAdmission: Send + Sync {
    async fn admit(&self, request: AdapterDeliveryRequest) -> Result<Admission, AdmitError>;
}
