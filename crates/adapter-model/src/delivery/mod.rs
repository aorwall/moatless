//! Durable delivery admission, records, and execution contracts.

mod admission;
mod error;
mod handlers;
mod models;
mod request;

pub use admission::DeliveryAdmission;
pub use error::{AdapterDeliveryFailure, AdmitError};
pub use handlers::{AdapterDeliveryHandler, AdapterDeliveryHandlers};
pub use models::{AdapterDelivery, AdapterDeliveryRequest, Admission};
pub use request::inbound_delivery_request;
