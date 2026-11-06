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
| Backend Memory High | working set above 70% of the limit for 10m | the same memory series; whether it climbs steadily (a leak) or in steps (bursts) | moatless |
| Deployment Unavailable | backend, t3, cliproxy or traefik has 0 available replicas for 2m | `kubectl -n moatless describe deploy/<name>`, events, image pull errors | needs a person, or gitops |
| Container Restarted | a non-sandbox container restarted, except a backend OOM kill (Backend OOMKilled covers that) | `kubectl logs --previous`; the exit code in `kubectl describe pod` | moatless |
| Pod CrashLoopBackOff | a non-sandbox container is crash-looping for 2m | `kubectl logs --previous` | moatless or gitops |
| Backend Metrics Absent | no `moatless_database_pool_connections` series for 10m | is the backend up; is the scrape annotation still on the pod | gitops or needs a person |
| ALB 5xx | any 5xx from the `moatless-ingress` load balancer itself in a minute | backend availability at that minute; ALB target health | usually a symptom of another alert |

## Database

| Rule | Fires when | Look first | Fix usually in |
|---|---|---|---|
| Database Pool Saturated | a backend pod held 75% or more of its pool in every minute for 3m | `moatless_database_pool_connections{state="in_use"}` by pod; `moatless_ui_rpc_poll_gate_wait_seconds`; Loki `|= "pool timed out"` | moatless |
| Database Readiness Failing | any readiness failure in 5m; the probe has its own connection, so this means the database is unreachable | `moatless_database_readiness_failures_total` by `reason`; Aurora `aws_rds_*` metrics | moatless, or needs a person if Aurora is down |

A pool spike that lasts under 3 minutes is below this rule's threshold. The rule
ignores a pod's first 5 minutes, when it fills its pool after a rollout.

## Adapters (Slack, GitHub, Linear webhooks)

| Rule | Fires when | Look first | Fix usually in |
|---|---|---|---|
| Adapter Delivery Stalled | the oldest pending delivery is older than 3m for 2m | `moatless_adapter_delivery_oldest_pending_seconds`; delivery worker logs | moatless |
| Adapter Delivery Failed | a delivery ended `terminal` in 15m | Loki for the adapter and kind on the `asset` label | moatless |
| Adapter Delivery Rejected | admitted outcome `error` or `at_capacity` in 15m | `moatless_adapter_delivery_admitted_total` by outcome | moatless |
| Adapter API Errors | in 15m: 5 `client_error`, or 1 `server_error`, `error` (older pods) or `auth_error` call to an adapter's API | `client_error` is a caller's bad request; `server_error` the provider failing or throttling; `auth_error` usually an expired or revoked credential | the caller, needs a person (credential), or moatless |

## Sandboxes and agents

| Rule | Fires when | Look first | Fix usually in |
|---|---|---|---|
| Sandbox Provisioning Failing | any failed provision in 30m | `moatless_sandbox_provision_duration_seconds_count` by outcome; backend logs for the task ids | moatless, or needs a person for node capacity |
| Loop Task Refused | a Loop firing was refused at task preflight, so no agent ran | `moat loops tasks --id <loop_id>`, then `moat events list <task>` for the `preflight_failed` message | moatless when the refusal is identity or ownership; the Loop config or a credential when it names a token or repository |
| Workspace Sync Failing | any failed workspace upload or download to S3 in 30m | `moatless_workspace_sync_total` by direction and outcome | moatless |
| Sandbox Evicted | a `moatless-ws-*` pod became `Evicted` | `kubectl describe` the pod: ephemeral storage or memory pressure | the `sandbox-troubleshooting` skill; gitops for sandbox resources |
| Sandbox OOMKilled | a `moatless-ws-*` container ran out of memory | which workspace and task the pod belongs to; the `workspace-resource-tuning` skill, if available | gitops or the workspace's resources, rarely moatless |
| Model Credentials Exhausted | every credential the model hub (cliproxy) holds for a model is in cooldown, so agents on it stall or fail | cliproxy logs `\|= "are in cooldown"` for the reason and `remaining`; which Loops run that model | needs a person: add a credential or move the Loops to another agent |
| Agent Runs Failing | more than 20% of agent runs errored in 1h, and at least 3 | `moatless_agent_runs_total` by `agent_profile`; LLM provider status | moatless, or the provider |

## UI and logs

UI-RPC Defects and Error Log Burst start a fix only on their second firing in
24h.

| Rule | Fires when | Look first | Fix usually in |
|---|---|---|---|
| UI-RPC Defects | a UI-RPC method (`tag`) died at least once in 15m | Loki `target="moatless_t3code::rpc::errors"` for that tag | moatless |
| UI-RPC Slow | UI-RPC p95 above 2s for 10m | `moatless_ui_rpc_request_duration_seconds` by `tag`; database pool and poll gate waits | moatless |
| Error Log Burst | 5 or more backend ERROR lines in 5m | Loki `{namespace="moatless", service_name="moatless", log_level="ERROR"}` grouped by `target` and `message` | moatless |
