//! Backend dependency assembly.

pub mod adapter_config;
pub(crate) mod adapter_installations;
mod ports;
pub use ports::{Ports, compose};

pub(crate) mod application;
pub(crate) mod http;
pub(crate) mod infrastructure;
pub(crate) mod secrets;
pub(crate) mod workers;

pub(crate) mod sandbox;
