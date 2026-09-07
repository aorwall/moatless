/// Register Prometheus metric descriptions for event admission and the relay.
pub fn describe_ingress_metrics() {
    metrics::describe_counter!(
        "event_ingress_admitted_total",
        "Sandbox events durably admitted, by durability kind"
    );
    metrics::describe_counter!(
        "event_ingress_relay_republished_total",
        "Ingress outbox records republished to the consumer path by the relay"
    );
    metrics::describe_counter!(
        "event_ingress_relay_failures_total",
        "Ingress outbox records the relay failed to republish"
    );
    metrics::describe_counter!(
        "event_ingress_delivered_total",
        "Ingress outbox records deleted after their consumer committed them"
    );
    metrics::describe_gauge!(
        "event_ingress_outbox_pending",
        "Ingress outbox records awaiting confirmed delivery"
    );
    metrics::describe_gauge!(
        "event_ingress_outbox_stalled",
        "Ingress outbox records undelivered for far longer than one retry"
    );
}
