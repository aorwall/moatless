# Metrics diagnosis

Use metrics for fleet trends and logs/events for the individual failed task.
Discover the backend metrics listener and service port from the running deployment;
application port 8080 and metrics port 9090 are distinct defaults. Use an explicit
context and namespace if port-forwarding. Scrape `/metrics` on the metrics listener.
A missing series can mean no observation yet, not a zero-valued event.

| Signal | Source of metric names and labels |
|---|---|
| Webhook outcomes and Sender API calls | [adapter-model metrics](../../../../crates/adapter-model/src/metrics.rs) |
| Binding sends | [binding metrics](../../../../crates/loops/src/bindings/metrics.rs) |
| Delivery admission, execution, backlog and oldest pending age | [delivery metrics](../../../../crates/loops/src/adapters/delivery/metrics.rs) |
| Scheduled Loop firings | [firing metrics](../../../../crates/loops/src/loops/firing/metrics.rs) |

Useful families include `moatless_adapter_webhook_total`,
`moatless_adapter_api_total`, `moatless_binding_outbound_total`, and
`moatless_loop_schedule_firing_total`. Read the definitions before filtering labels.
For sandbox and agent families, locate their current emitters with:

```bash
rg -n 'counter!|histogram!|gauge!' crates/sandbox* crates/tasks crates/events
```

For a delivery stall, compare admitted versus executed rates, backlog, and oldest
pending age; inspect worker logs for retry/permanent failures. For resource failures,
correlate runtime terminations with node pressure and per-container usage. Metrics
cannot prove a specific message was delivered; verify the task transcript or Binding.
