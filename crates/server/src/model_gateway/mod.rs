//! The backend's model gateway.
//!
//! A pod's agent calls `/llm/{provider}/…` with its task token as the API key.
//! The gateway resolves the model credential the task's actor holds and streams
//! the provider's answer back unchanged, status and rate-limit headers
//! included, so the pod's classifier still recognises a usage limit.

mod anthropic;
mod auth_mode;
#[cfg(test)]
mod fakes;
mod openai;
mod routes;
mod service;
mod stream;

pub use routes::model_gateway_router;
pub use service::{ModelGateway, ProviderBases};
