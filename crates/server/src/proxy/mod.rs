//! What the backend's sandbox-facing proxies share: verifying the pod's task
//! token, and forwarding a request upstream without leaking the hop's own
//! headers.

pub(crate) mod caller;
pub(crate) mod forward;
