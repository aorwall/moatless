//! Slack's kinds on the durable Adapter delivery seam.
//!
//! Binding `INBOUND_MESSAGE` to its `InboundMessageHandler` happens at the
//! composition root (`backend/src/adapter_installations.rs`), the one crate
//! that holds both the `RoutingService` the handler needs and the
//! `moatless-loops` band it comes from. What stays here is the vocabulary the
//! rest of this crate reads: the adapter name and its one delivery kind.

/// Adapter name on `adapter_delivery` rows, metrics, and bindings.
pub const ADAPTER: &str = "slack";

/// Routing one inbound Slack message.
///
/// Namespaced under `slack.` so kinds stay owned by exactly one Adapter and
/// two migrations never contend for the same name.
pub const INBOUND_MESSAGE: &str = "slack.inbound_message";

#[cfg(test)]
mod tests {
    use super::*;

    /// The kind is half of the durable dedup key and is matched by string at
    /// both ends, so renaming it orphans every record already admitted.
    #[test]
    fn the_kind_is_namespaced_to_this_adapter() {
        assert_eq!(INBOUND_MESSAGE, "slack.inbound_message");
        assert!(INBOUND_MESSAGE.starts_with(&format!("{ADAPTER}.")));
    }
}
