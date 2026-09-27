/// Register Prometheus metric descriptions for event admission.
pub fn describe_ingress_metrics() {
    metrics::describe_counter!(
        "event_ingress_admitted_total",
        "Sandbox events durably admitted, by durability kind"
    );
}
