//! Durable admission for every inbound Adapter delivery.
//!
//! A handler commits an `adapter_delivery` record keyed by `(adapter, kind,
//! event_id)` before answering the sender; `adapter-delivery-worker` claims
//! and executes it, one record per fanned-out kind.
//!
//! Surface `AdmitError` to the sender as retryable — a 200 over it drops the
//! delivery. No in-process dedup cache: it would 200 a redelivery whose
//! admission failed, with nothing durable behind it.

mod metrics;
mod models;
mod typed;
mod worker;

pub use metrics::describe_adapter_delivery_metrics;
pub use models::AdapterDeliveryPolicy;
pub use typed::{ConnectionDeliveryPayload, register_connection_delivery, register_typed_delivery};
pub use worker::AdapterDeliveryWorker;

pub(crate) mod dao;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
