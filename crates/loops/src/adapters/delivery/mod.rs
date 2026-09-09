//! Durable admission for every inbound Adapter delivery.
//!
//! A handler commits an `adapter_delivery` record keyed by
//! `(adapter, kind, event_id)` before answering the sender; the supervised
//! `adapter-delivery-worker` claims and executes it. One sender delivery that
//! fans into several kinds becomes one record per kind, each on its own budget.
//!
//! Surface an `AdmitError` to the sender as a retryable failure — a 200 over it
//! drops the delivery. Do not put an in-process dedup cache in front of
//! admission: it would answer a redelivery whose admission failed with a 200 and
//! nothing durable behind it.

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
