//! Backend dependency assembly.

mod ports;
pub(crate) mod adapter_installations;
pub mod adapter_config;
pub use ports::{Ports, LifecycleOptions, compose, github_app_registry};

pub(crate) mod application;
pub(crate) mod workers;
pub(crate) mod http;
pub(crate) mod secrets;
pub(crate) mod infrastructure;

pub(crate) mod sandbox;
