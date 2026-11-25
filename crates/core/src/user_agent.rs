//! The product string every outbound HTTP client presents.

/// The `User-Agent` this deployment sends to a git host.
///
/// `reqwest` sends none unless the client is built with one, and a self-hosted
/// instance behind a proxy that filters on the header answers 403 to every
/// request that carries none.
pub const USER_AGENT: &str = "moatless";
