//! Maintain per-Task runtime context inside a shared sandbox.

mod checkout;
mod models;
mod registration;
pub use models::{CheckoutMode, RegisterTaskContextError, TaskContext, TaskContextLookupError};
pub(crate) use registration::register;
