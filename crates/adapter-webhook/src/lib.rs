//! Webhook Adapter: generic HTTP inbound delivery and callback outbound.
//!
//! The backend remains the composition root and reaches this crate through a
//! permanent facade module
//! (`crate::adapters::providers::webhook`), so the server band keeps naming its
//! existing paths unchanged.

// Allow unwrap/expect in test code. Production code still enforces the deny.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod adapter;
pub mod delivery;
pub mod external_id;
pub mod handlers;
pub mod signature;
