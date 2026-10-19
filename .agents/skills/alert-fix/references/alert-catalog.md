# Moatless alert catalog

Every rule in the `Moatless Alerts` Grafana folder, all routed to `#alerts-moatless`
by the `route=moatless` label. The queries are in soap-infra,
`terraform/platform/grafana/modules/grafana-alerts/moatless.tf`; read the current
query there before you call a rule wrong, because this table summarises it.

The backend runs 2 replicas (`moatless-vibe-backend`), each with a 20-connection
database pool and a 1536Mi memory limit. Sandbox pods are named `moatless-ws-*`.

## Platform

| Rule | Fires when | Look first | Fix usually in |
|---|---|---|---|
| Backend OOMKilled | a backend container's last termination was `OOMKilled` | `container_memory_working_set_bytes{container="backend"}` by pod over the last hour; the UI-RPC and `auth/verify` request rate in the minute before | moatless (backend memory per request) |
| Backend Memory High | working set above 85% of the limit for 15m | the same memory series; whether it climbs steadily (a leak) or in steps (bursts) | moatless |
| Deployment Unavailable | backend, t3, cliproxy or traefik has 0 available replicas for 2m | `kubectl -n moatless describe deploy/<name>`, events, image pull errors | needs a person, or gitops |
| Pod CrashLoopBackOff | a non-sandbox container is crash-looping for 2m | `kubectl logs --previous` | moatless or gitops |
| Backend Metrics Absent | no `moatless_database_pool_connections` series for 10m | is the backend up; is the scrape annotation still on the pod | gitops or needs a person |
| ALB 5xx | 5 or more 5xx from the `moatless-ingress` load balancer in a minute | backend availability at that minute; ALB target health | usually a symptom of another alert |

## Database

| Rule | Fires when | Look first | Fix usually in |
|---|---|---|---|
| Database Pool Saturated | a backend pod held 90% or more of its pool in every minute for 5m | `moatless_database_pool_connections{state="in_use"}` by pod; `moatless_ui_rpc_poll_gate_wait_seconds`; Loki `|= "pool timed out"` | moatless |
| Database Readiness Failing | readiness failures in every minute for 5m | `moatless_database_readiness_failures_total` by `reason`; Aurora `aws_rds_*` metrics | moatless, or needs a person if Aurora is down |

A pool spike that lasts under 5 minutes is below this rule's threshold and is
expected right after a rollout.

## Adapters (Slack, GitHub, Linear webhooks)

| Rule | Fires when | Look first | Fix usually in |
|---|---|---|---|
| Adapter Delivery Stalled | the oldest pending delivery is older than 10m for 5m | `moatless_adapter_delivery_oldest_pending_seconds`; delivery worker logs | moatless |
| Adapter Delivery Failed | a delivery ended `terminal` in 15m | Loki for the adapter and kind on the `asset` label | moatless |
| Adapter Delivery Rejected | admitted outcome `error` or `at_capacity` in 15m | `moatless_adapter_delivery_admitted_total` by outcome | moatless |
| Adapter API Errors | 5 or more `error` or `auth_error` calls to an adapter's API in 15m | `auth_error` is usually an expired or revoked credential | needs a person (credential), or moatless |

## Sandboxes and agents

| Rule | Fires when | Look first | Fix usually in |
|---|---|---|---|
| Sandbox Provisioning Failing | 3 or more failed provisions in 30m | `moatless_sandbox_provision_duration_seconds_count` by outcome; backend logs for the task ids | moatless, or needs a person for node capacity |
| Workspace Sync Failing | 3 or more failed workspace uploads or downloads to S3 in 30m | `moatless_workspace_sync_total` by direction and outcome | moatless |
| Sandbox Evicted | a `moatless-ws-*` pod became `Evicted` | `kubectl describe` the pod: ephemeral storage or memory pressure | the `sandbox-troubleshooting` skill; gitops for sandbox resources |
| Agent Runs Failing | more than half of agent runs errored in 1h, and at least 5 | `moatless_agent_runs_total` by `agent_profile`; LLM provider status | moatless, or the provider |

## UI and logs (start a fix only on the second firing in 24h)

| Rule | Fires when | Look first | Fix usually in |
|---|---|---|---|
| UI-RPC Defects | a UI-RPC method (`tag`) died 5 or more times in 15m | Loki `target="moatless_t3code::rpc::errors"` for that tag | moatless |
| Error Log Burst | 25 or more backend ERROR lines in 5m | Loki `{namespace="moatless", service_name="moatless", log_level="ERROR"}` grouped by `target` and `message` | moatless |
