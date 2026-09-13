//! The external event id a webhook delivery carries downstream.

/// `webhook:delivery:<delivery_id>` — one inbound delivery.
///
/// Guards a second HTTP delivery, not a second execution: the worker is
/// at-least-once, so routing looks this id up against messages already
/// delivered to make a re-run converge. Minted once at parse time and stored,
/// never recomputed per attempt.
pub(crate) fn delivery(delivery_id: &str) -> String {
    format!("webhook:delivery:{delivery_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id is matched against ids already stored on a task, so its shape is
    /// part of the durable contract: changing it makes every record admitted
    /// under the old shape re-deliver on its next execution.
    #[test]
    fn a_delivery_id_is_namespaced_to_this_adapter() {
        assert_eq!(delivery("order-4711"), "webhook:delivery:order-4711");
    }

    /// Two deliveries are two external ids — otherwise the second would be
    /// discarded as a repeat of the first.
    #[test]
    fn two_deliveries_do_not_share_an_external_id() {
        assert_ne!(delivery("one"), delivery("two"));
    }
}
