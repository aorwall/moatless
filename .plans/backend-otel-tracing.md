# OpenTelemetry tracing for the Moatless backend, exported to Tempo

> **Status:** approved
> **Updated:** 2026-08-02
> **Vocabulary:** CONTEXT.md
> **Sibling spec:** `.plans/sandbox-fleet-diagnostics.md` (independent; ships separately)

## Current solution

The backend logs but does not trace. `init_tracing` installs a single JSON `fmt` layer to
stdout:

- `backend/src/startup.rs:44-59` — `tracing_subscriber::fmt().with_env_filter(...).json()
  .flatten_event(true).with_current_span(true).with_span_list(false).init()`. The stdout
  JSON is shipped to **Loki** by an external cluster agent (Promtail/Alloy — not in this
  repo). `.with_current_span(true)` renders the *current span's fields* under a `span`
  object, but there is **no `trace_id`/`span_id`** on the line.
- There is **no `opentelemetry` / `opentelemetry-otlp` / `tracing-opentelemetry` crate**
  anywhere (verified across `Cargo.toml`, `backend/Cargo.toml`, `backend/src`).
- Metrics are separate and fine: `metrics_exporter_prometheus` on `/metrics`
  (`backend/src/metrics.rs:15`, `startup.rs:70`). This spec does not touch metrics.
- The Helm chart has Prometheus + Grafana **dashboards** only — `backend-servicemonitor.yaml`,
  `podmonitor.yaml`, `prometheusrule.yaml`, `grafana-dashboard-configmap.yaml`. **No Loki,
  no Tempo, no OTel collector** templates; Loki is an external target of the JSON logs.

**The trace seam already exists and is deliberately dropped:**

- The T3 client sends trace context on **every** RPC request frame:
  `ClientFrame::Request { id, tag, payload, headers, traceId, spanId, sampled }`
  (`backend/src/ui_rpc/mod.rs:1287-1305`).
- The handler parses and **throws it away**: `drop((headers, trace_id, span_id, sampled))`
  with the comment *"Parsing these fields is intentional even though phase one does not
  consume request headers or tracing context"* (`backend/src/ui_rpc/mod.rs:1436-1438`).
- The backend reports tracing off: `"otlpTracesEnabled": false` in the config descriptor
  (`backend/src/ui_rpc/mod.rs:2611`).

**What the code does not say:**

- **Instrumentation is nearly absent.** Only ~8 `#[instrument]` sites exist backend-wide
  (mostly `backend/src/sandbox/grpc_server.rs`). Useful traces require adding spans to the
  operations worth following — this is real work, not a free harvest from existing spans.
- **The backend runs ≥2 replicas** (`k8s/helm/moatless-vibe/values.yaml:270`,334; HPA
  min 2, max 6). A T3 client's `/ws` connection is pinned to one replica. This is why
  trace *storage on the backend* was rejected in favour of exporting to Tempo — a central
  store sidesteps the per-replica problem entirely.
- **The client trace-id/span-id format** must be confirmed on the T3 side before wiring
  context extraction. T3 uses Effect's tracer with an optional OTLP delegate
  (`t3code/packages/shared/src/observability.ts:447`), so its `traceId`/`span_id` are
  expected to be OTel-compatible hex ids — verify against
  `t3code/packages/contracts` / the client frame construction and extract accordingly
  (W3C-style: a 16-byte trace id, 8-byte span id). If the format is not directly usable,
  mint a fresh root span id and record the client value as an attribute rather than
  failing.

## Planned solution

Add real OpenTelemetry tracing to the backend: build a request-root span from the trace
context the client already sends, propagate it through instrumented operations, export
spans over OTLP to Tempo for waterfalls in Grafana, and stamp `trace_id`/`span_id` into
the JSON logs so Loki logs correlate to traces. Config-driven and off unless an OTLP
endpoint is set.

**New**
- OTel dependencies: `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`,
  `tracing-opentelemetry` (workspace `Cargo.toml` + `backend/Cargo.toml`).
- An OTel tracer/OTLP exporter and a `tracing-opentelemetry` layer, added beside the JSON
  `fmt` layer in `init_tracing`.
- Trace-context extraction at the RPC/REST boundary, building a request-root span from the
  client's `traceId`/`spanId`/`sampled`.
- `#[instrument]` spans on the operations worth tracing.
- Config for the OTLP endpoint + sampling ratio; Helm/values plumbing for it; a Grafana
  Tempo datasource with logs↔traces correlation.

**Changes materially**
- `backend/src/startup.rs:44` — `init_tracing` becomes a layered `Registry` (fmt layer +
  OTel layer), with `trace_id`/`span_id` added to log output.
- `backend/src/ui_rpc/mod.rs:1436` — the dropped trace context is consumed.
- `backend/src/ui_rpc/mod.rs:2611` — `otlpTracesEnabled` reflects whether export is on.

**Untouched**
- The Prometheus metrics pipeline and existing Grafana dashboards.
- The stdout-JSON → Loki log format (fields are added, the transport is unchanged).
- Sandbox lifecycle, reconciler, and all product behaviour.
- The T3 client — it already sends the context; nothing on that side must change for this
  to work (the T3 diagnostics UI is the sibling spec, unrelated).

### Problem

When a request misbehaves, an operator has only disconnected log lines: nothing ties a
request's logs together across the handlers and background work it touches, and there is
no span timing. The client already offers a `trace_id`; the backend discards it. There is
no way to see a request as a trace in Grafana.

### Behaviour

- **As an operator**, I open Grafana Tempo, search by `trace_id` (or find a slow request),
  and see a waterfall of the backend spans for that request, with timings.
- **From a Loki log line**, I can pivot to its trace: every line carries `trace_id`
  (and `span_id`), and a Grafana derived field links it to the Tempo trace.
- **A trace started in the T3 client continues into the backend** — the request-root span
  is a child of the client-provided context, so the client and backend spans share one
  `trace_id`.
- **With no OTLP endpoint configured**, the backend behaves exactly as today (logs only,
  no exporter, `otlpTracesEnabled: false`), so existing/Docker deployments are unaffected.

### Architecture and key flows

- **`init_tracing` (`backend/src/startup.rs:44`)** becomes a `tracing_subscriber::registry()`
  composing: (1) the existing JSON `fmt` layer — extended so each event carries `trace_id`
  and `span_id` (via `tracing-opentelemetry`'s span context, e.g. a small custom field
  formatter or `OpenTelemetrySpanExt`); (2) a `tracing_opentelemetry::layer()` bound to an
  OTLP-exporting `TracerProvider`. The exporter is built only when an OTLP endpoint is
  configured; otherwise the OTel layer is omitted and behaviour is unchanged. Keep the
  `EnvFilter` and JSON output for Loki.
- **Exporter/provider**: `opentelemetry-otlp` (gRPC or HTTP — match the Tempo distributor's
  accepted protocol; default OTLP/gRPC `4317`) with a batch span processor, a `service.name`
  resource of the backend, and a sampler (see below). Own its shutdown in the root
  supervisor so buffered spans flush on exit.
- **Context extraction at the boundary** (`backend/src/ui_rpc/mod.rs:1436`, and the
  equivalent for REST handlers if in scope): replace the `drop(...)` with code that builds
  a remote `SpanContext` from `trace_id`/`span_id`/`sampled` and starts the handler's
  root span as its child, so client and backend join one trace. Guard bad/absent values by
  minting a fresh root and attaching the raw client value as an attribute.
- **Instrumentation set** (add `#[instrument]`, keeping fields low-cardinality per the
  metrics-cardinality convention in `backend/src/event_handler/metrics.rs:9`): the UI-RPC
  request dispatch (`mod.rs` `handle_request`, one span per `tag`), the REST handlers under
  `backend/src/sandbox/handlers.rs` and other primary routes, agent-turn handling in the
  event handler, and sandbox lifecycle/K8s operations
  (`backend/src/sandbox/lifecycle/`, `k8s/`). Start focused; breadth can grow later.
- **`otlpTracesEnabled`** (`mod.rs:2611`) is computed from whether the exporter is active,
  so the T3 client's `ServerObservability` reflects reality.
- **Grafana**: add a Tempo datasource and a Loki→Tempo derived field on `trace_id`
  (Helm/values, alongside the existing dashboard configmaps). Deploying Tempo itself is an
  operational choice — see Failure/config below.

Key flow (client → Tempo): T3 sends `Request { traceId, spanId, sampled }` → the RPC
boundary builds a request-root span from that context → instrumented handlers create child
spans → on span close the batch processor exports OTLP to Tempo → Grafana shows the
waterfall; every log line for the request also carries `trace_id` into Loki.

### Data model

No database state. Configuration only:

`OtelConfig` (new, on the backend config struct)
| field | type | note |
|---|---|---|
| `otlp_endpoint` | `Option<String>` | env-driven (e.g. `OTEL_EXPORTER_OTLP_ENDPOINT`); `None` ⇒ tracing off |
| `sampling_ratio` | `f64` | head sampling ratio for the parent-based sampler; default e.g. `0.1` |
| `service_name` | `String` | OTel `service.name` resource; default `moatless-vibe-backend` |

Sampling is **parent-based, honouring the client's `sampled` flag**: if the incoming
context says sampled, keep it; otherwise apply the head ratio. This bounds span volume
without dropping traces the client already decided to keep.

### Interfaces and contracts

- **External (changed):** `otlpTracesEnabled` in the T3 `ServerObservability` payload
  (`mod.rs:2611`) — now true when export is on. The T3 client reads this; no client change
  required.
- **External (new):** OTLP span export to a Tempo endpoint, and `trace_id` in the Loki log
  stream. Consumed by Grafana (Tempo datasource + Loki derived field).
- **Internal (changed):** the RPC `ClientFrame::Request` trace fields
  (`mod.rs:1297-1304`) go from dropped to consumed. Not a wire change — the fields already
  exist on the frame.

### Failure, safety and recovery

| When | Behaviour | Wording fixed? |
|---|---|---|
| No `otlp_endpoint` configured | OTel layer omitted; logs-only, exactly as today; `otlpTracesEnabled: false` | n/a |
| Tempo unreachable | Exporter drops/retries per the batch processor; **request handling is never blocked or failed** by trace export | n/a |
| Malformed client trace context | Mint a fresh root span, record the raw client value as an attribute; never fail the request | n/a |
| Backend shutdown | Flush the batch span processor in the root supervisor before exit | n/a |

Tracing must be strictly best-effort and off the request's critical path — no export error
may surface to a user or fail an RPC. Rolling the change back = unset the OTLP endpoint;
the backend reverts to logs-only with no code change.

**Tempo deployment** is an operational decision left to the deployer: the backend only
needs an OTLP endpoint. If your clusters do not already run Tempo, add it (or an OTel
collector fronting it) to the Helm chart or point the endpoint at an existing collector.
The backend code must not assume Tempo is present — it activates only when the endpoint is
set.

### Testing and verification

- **Rust:** unit-test context extraction (client `trace_id`/`span_id`/`sampled` → a child
  root span sharing the trace id; malformed input → fresh root + attribute). Test that with
  no endpoint the tracer layer is absent and `otlpTracesEnabled` is false. If feasible,
  an in-memory/stdout span exporter test asserting a request produces a span tree with a
  shared trace id. Run `cargo clippy -p moatless-vibe-backend --all-targets -- -D warnings`
  and `cargo test -p moatless-vibe-backend`.
- **Manual (the part only a person judges):** point the backend at a Tempo/OTel-collector
  endpoint, drive a real request from T3, and confirm in Grafana that (a) the trace
  waterfall is legible and spans the handlers/sandbox ops touched, (b) the client and
  backend spans share one trace, and (c) a Loki log line for the request links back to the
  trace by `trace_id`.
- **Acceptance:** Given an OTLP endpoint is configured, When a T3 client makes a request,
  Then a trace with the client's `trace_id` appears in Tempo with backend child spans, and
  the request's log lines carry that `trace_id`; And Given no endpoint is configured, the
  backend behaves exactly as before.

## Out of scope

- Any T3-side UI for traces (no `getTraceDiagnostics`, no Moatless span store, no reused
  T3 trace panels). Traces live in Grafana.
- OpenTelemetry **metrics** — Prometheus stays as-is.
- Instrumenting the Sandbox process / agent CLI itself (backend-only here).
- The fleet sandbox diagnostics UI — that is `.plans/sandbox-fleet-diagnostics.md`.
