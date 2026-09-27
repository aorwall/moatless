//! Durable admission of sandbox-published events.

mod admission;
mod error;
mod grpc;
mod metrics;

pub use admission::{AdmissionRequest, EventAdmission, is_error_sub_type};
pub use error::AdmissionError;
pub use metrics::describe_ingress_metrics;

#[cfg(test)]
mod tests;
