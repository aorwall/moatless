//! The backend's model gateway.
//!
//! A pod's agent calls `/llm/{provider}/…` with its task token as the API key.
//! The gateway resolves the model credential the task's actor holds and streams
//! the provider's answer back unchanged, status and rate-limit headers
//! included, so the pod's classifier still recognises a usage limit.

mod anthropic;
mod routes;
mod service;
mod stream;

pub use routes::model_gateway_router;
pub use service::{ANTHROPIC_API_BASE, ModelGateway};
