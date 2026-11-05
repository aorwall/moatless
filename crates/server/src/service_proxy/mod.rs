//! The generic service proxy: `/proxy/{service}/…` forwards a pod's request
//! to an upstream `SERVICE_PROXIES_JSON` defines, with a credential the pod
//! never holds and only where the definition's rules allow.

mod access;
mod handlers;
mod routes;
mod service;

pub use access::{GrantReader, ServiceAccess};
pub use routes::service_proxy_router;
pub use service::ServiceProxyGateway;
