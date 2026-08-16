# Harden the sandbox against untrusted in-pod code

> **Status:** approved
> **Updated:** 2026-08-16
> **Vocabulary:** CONTEXT.md

The in-pod threat model is **fully untrusted code execution**: the agent runs with `SANDBOX_PERMISSION_MODE=bypassPermissions` by default (`sandbox/AGENTS.md`), so every control below treats the agent process as the adversary. The goal is defence in depth so that compromising the sandbox yields only task-scoped, low-value material.

## Current solution

Anchors — where each change lands:

- **gRPC control server** binds `0.0.0.0:50052` with a bare `tonic::transport::Server::builder()` — no `.tls_config()`, no auth interceptor. `sandbox/src/main.rs:251-260`.
- **Sandbox → backend channel** already has a Bearer-token `AuthInterceptor` with a runtime-swappable token. `sandbox/src/backend_client.rs:155-172` (interceptor), `:140` (token swap), `:181` (client wrapper).
- **Sandbox pod spec** sets `security_context: pod_security_context(config)` and **never sets `runtime_class_name`** → default host-kernel runtime. `crates/sandbox-k8s/src/pod_builder.rs:318`.
- **`pod_security_context`** returns the hardened context (uid 1000, seccomp `RuntimeDefault`) only when `config.sandbox.run_as_non_root`; otherwise `None` (root, no seccomp). `pod_builder.rs:1407`.
- **`app_security_context`** drops ALL caps, always adds `NET_BIND_SERVICE`, and **adds `DAC_OVERRIDE` when not run_as_non_root**. No `allow_privilege_escalation:false`, no `read_only_root_filesystem`. `pod_builder.rs:1381`.
- **Non-root is default off**: `sandboxRunAsNonRoot: false` → `K8S_SANDBOX_RUN_AS_NON_ROOT`. `k8s/helm/moatless-vibe/values.yaml:666`; consumed at `pod_builder.rs:1371`.
- **Pod volumes** are built inline (the `/run/moatless` tmpfs among them) — the pattern a cert mount follows. `pod_builder.rs:152-255`.
- **Privileged dind sidecar** — opt-in per repo (`dockerAccess`), default false; `privileged_security_context()`. `pod_builder.rs:243-280`, `:1436`.
- **External secrets are materialized in the sandbox today**: `sandbox/src/init/credentials.rs` writes `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `CODEX_AUTH_JSON` / `GITHUB_TOKEN` into the agent's env and auth files (`:365`, `:473`, `:815`); resolved by `crates/sandbox-control/src/secrets.rs`; keys enumerated at `crates/sandbox-control/src/grpc_server.rs:77-78`. `docker/docker-entrypoint-sandbox.sh` writes `CODEARTIFACT_AUTH_TOKEN` into `~/.npmrc` and Maven `settings.xml` and exports `PIP_INDEX_URL` with the token in the URL.
- **Credential broker** — unix socket `/run/moatless/credentials.sock`, mints a fresh token per call. `sandbox/src/credentials/broker.rs`; helper `docker/moatless-git-cred.sh`.
- **Egress** is fail-closed (squid allowlist, **no TLS termination** — filters on CONNECT host / SNI). `k8s/helm/moatless-vibe/templates/egress-proxy.yaml`. **Ingress** to `:50052` restricted to backend + Traefik. `templates/sandbox-networkpolicy.yaml`.

What the code does not say:

- The supervisor and the agent subprocess **share one container and one uid** (`sandbox/AGENTS.md`, load-bearing rule). A non-root pod stops cross-uid `/proc/<pid>/mem` from the node, but not the agent reading the co-located supervisor at the same uid, and not the agent hitting the broker socket as a same-uid peer (`SO_PEERCRED` cannot distinguish same-uid callers).
- A second container in the same pod is a weak boundary: it shares the network namespace, Pod IP, lifecycle and usually volumes. Splitting the supervisor off into its own container turns `spawn(agent)` into an RPC and makes the execution side a second supervisor — cost without a real boundary. The isolation boundary is the sandbox itself, not an in-pod container split.
- The admin-only override for privileged sandbox fields is referenced as `ensure_privileged_sandbox_fields` in `k8s/helm/moatless-vibe/README.md:502` — confirm the exact site in `crates/sandbox-control/src` before wiring the root override through it.

## Planned solution

**New**

- mTLS on the `:50052` server plus a task-token auth interceptor (`sandbox/src/main.rs:251-260`; new interceptor module).
- A per-task identity carrying a **sandbox-scoped capability** (this task only), minted by the backend, issued as an mTLS cert (cert-manager) + a short-lived capability token.
- `runtime_class_name = "gvisor"` on the sandbox PodSpec, gated by config, plus a runsc RuntimeClass and node pool.
- `SO_PEERCRED` peer check on the broker socket (`sandbox/src/credentials/broker.rs`).

**Changes materially**

- `config.sandbox.run_as_non_root` default → true; `sandboxRunAsNonRoot: true` (`values.yaml:666`).
- `app_security_context` (`pod_builder.rs:1381`): `DAC_OVERRIDE` gone from the default path; add `allow_privilege_escalation:false` + `read_only_root_filesystem:true` with writable mounts.
- Sandbox → backend channel (`backend_client.rs`) presents the mTLS identity and carries the **sandbox-scoped capability token** instead of a long-lived token; nothing long-lived is cached in the daemon.
- Agent subprocess spawns under a **distinct uid** from the supervisor (`sandbox/src/agent/harness/`).
- `init/credentials.rs` stops exporting LLM/registry secrets into the agent's ambient shell env; `docker-entrypoint-sandbox.sh` tightens auth-file perms.

**Untouched**

- Credential broker socket protocol and per-call minting — only what the daemon caches and the peer-check change.
- Egress allowlist + ingress NetworkPolicy (kept as defence in depth).
- HTTP `:8001` health probes (stay unauthenticated).
- dind opt-in default (stays false).

### Problem

An agent running untrusted code beside the supervisor can today: read the supervisor's memory and the broker socket as a same-uid peer; read the real LLM, GitHub and registry secrets materialized in its own env and auth files; and — if it reaches the network or a kernel bug — find no isolation runtime under it and an unauthenticated control plane behind only a NetworkPolicy. Root mode makes all of this worse and is the default.

### Behaviour

- **Backend operator** drives a sandbox over `:50052`: the call now requires a valid client certificate **and** a capability token bound to that task; a peer with neither, or with another task's token, is rejected.
- **Sandbox daemon** calls the backend: it authenticates with its per-task mTLS identity and a short-lived capability token; on token expiry it refreshes, and on identity failure the channel fails closed rather than falling back.
- **Agent (adversary)** that reads the daemon's memory or auth files obtains only material scoped to its own task; it cannot reach another task, the database, platform secrets, or Kubernetes.
- **Admin** may still opt a workspace back to root for a repo that genuinely needs it; a non-admin cannot request it.

### Architecture and key flows

**Layer 1 — control-channel auth.** `sandbox/src/main.rs:251-260`: add `ServerTlsConfig` (server identity + client-CA root) to the builder so `:50052` requires a client cert. Add a new tonic interceptor (e.g. `sandbox/src/grpc_auth.rs`) that reads the capability token from call metadata and validates it against `TASK_ID` and the allowed capability set; reject on mismatch. The backend's client to `:50052` (probe the calling crate under `crates/`) presents its client cert and the token. Identities come from a **cert-manager per-namespace Issuer**: a `Certificate` is minted per task into a `Secret`, mounted into the pod following the volume pattern at `pod_builder.rs:152-255`; server reads cert/key/CA from those paths. The sandbox → backend side reuses the existing `AuthInterceptor` (`backend_client.rs:155-172`) for the token and adds client-cert mTLS on the `Channel`.

**Layer 2 — kernel isolation.** `pod_builder.rs` near `:318`: set `runtime_class_name: config.sandbox.runtime_class.clone()` (new config field + `K8S_SANDBOX_RUNTIME_CLASS`), `None` when unset. Install a `gvisor` (runsc) RuntimeClass and a node pool with a matching `nodeSelector` + toleration (new helm template + values). Where dind is resolved (`pod_builder.rs:243-280`), a repo with `dockerAccess=true` must **not** get the gVisor runtime (privileged dind is incompatible) — fall back to the default runtime for those pods. Roll out in gVisor's audit/soft posture first to harvest syscall gaps.

**Layer 3 — minimize in-sandbox trust.** The sandbox → backend token becomes a **capability token** the backend mints per task with a short TTL and a narrow claim set (report this task's output; read this task's info; expose this task's preview ports; mark this task complete). The backend authorizes every `BackendSandboxService` call against the token's `task_id`. No `USER_AUTH_TOKEN` / long-lived token is cached in the daemon; rotation uses the existing swap at `backend_client.rs:140`. The broker (`credentials/broker.rs`) gains an `SO_PEERCRED` check, serves only the agent's uid, and never returns the capability token — only per-call scoped external credentials.

**Layer 4 — baseline hygiene.** Flip `config.sandbox.run_as_non_root` and `values.yaml:666` to default true. In `app_security_context` (`pod_builder.rs:1381`) stop adding `DAC_OVERRIDE` on the default path (retain only under the admin root override), and add `allow_privilege_escalation:false` + `read_only_root_filesystem:true`; add `emptyDir`/tmpfs mounts for the writable paths (`/tmp`, `$CLAUDE_CONFIG_DIR`, `$CODEX_HOME`, `/run/moatless`; the workspace PVC is already writable). Spawn the agent under a distinct uid with no `CAP_SYS_PTRACE`, no privilege escalation, seccomp — defence in depth, not the fundamental control. Keep the root override admin-gated via the privileged-sandbox-field check.

**Layer 5 — cheap secret-hygiene wins (target-setting only).** In `init/credentials.rs` (`:365`, `:473`, `:815`) deliver LLM keys through the CLI's own auth file with `0600` perms instead of exporting `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` into the agent's ambient shell env. In `docker-entrypoint-sandbox.sh` `chmod 600` `~/.npmrc` and `~/.m2/settings.xml`, and scope `CODEARTIFACT_AUTH_TOKEN` to the setup-command subshell rather than exporting it into the environment the agent inherits. The target — no real external secret materialized in the sandbox — is fully realized by the follow-up injecting-proxy spec (Out of scope).

### Data model

The per-task identity is the only new stateful shape.

- **Capability token** — minted by the backend, short TTL, claims: `task_id`, `capabilities` (the closed set above — a client switches on it exhaustively), `exp`, `aud`. Held only in the daemon's memory, swapped on rotation; never persisted to a location that outlives the task or is mounted into a sidecar.
- **Task mTLS cert** — SAN carries the task identity the `:50052` interceptor keys on. Per-task, short lifetime (hours), renewed by cert-manager. On expiry the channel fails closed.

### Interfaces and contracts

- **External contract (breaking):** the backend ↔ sandbox gRPC control channel now requires mTLS **and** a capability token in **both** directions. The backend's client to `:50052` must present a client cert + token; the sandbox's `BackendSandboxServiceClient` must present its cert + capability token. Backend and sandbox deploys must be coordinated — an old backend cannot drive a new sandbox and vice versa. Stage behind a config flag so the plaintext path can be retired after both sides ship.
- **Internal interface (not a promise):** the `:50052` auth interceptor and the capability-token claim shape are implementation boundaries; they may change without a spec update as long as the external contract above holds.
- **Unchanged:** HTTP `:8001` health probes stay unauthenticated.

### Failure, safety and recovery

- **Cert missing/expired, or token invalid** → backend cannot drive the sandbox and the sandbox cannot call the backend. Fail closed with a clear status; never fall back to plaintext or to a long-lived token.
- **gVisor syscall gap breaks a workload** → caught in audit-mode rollout; per-namespace disable via `config.sandbox.runtime_class`; dind repos already run off gVisor.
- **Broker peer-check false negative** → the agent's git/registry call fails with a clear error; no fallback to the capability token.
- **Read-only rootfs missing a writable path** → the agent or CLI crashes on first write; enumerate the writable mounts and cover with the golden-manifest + smoke tests below.
- **Non-root default flip on an un-migrated image** (root-owned runtime paths) → pod fails to start; gate the rollout, admin override remains for lagging images.

### Testing and verification

- **Unit** — the `:50052` interceptor rejects a missing token, an invalid token, and a token for another task; accepts a valid one. mTLS client-cert requirement on the `Channel`.
- **Golden-manifest** (extend the existing `pod_builder` tests around `crates/sandbox-k8s/src/pod_builder.rs:2999+`) — a rendered non-dind sandbox pod has `runtimeClassName=gvisor`, non-root uid 1000, no `DAC_OVERRIDE`, `allowPrivilegeEscalation:false`, `readOnlyRootFilesystem:true`, a distinct agent uid, and the cert volume mounted; a dind pod has no `runtimeClassName`.
- **Integration / e2e** — a peer without cert + token cannot call `:50052`; a representative repo's setup + one agent turn completes under gVisor (the audit-mode harvest); the agent's `env` and auth-file reads do not reveal the capability token, and LLM/registry keys are absent from the ambient env.
- **Minimal-trust assertion** — carve the supervisor's heap and assert no long-lived backend token is present, only task-scoped material.
- **Commands** — `cargo build -p moatless-sandbox`, `cargo test -p moatless-sandbox`, and the sandbox-k8s crate's tests (confirm its package name in `crates/sandbox-k8s/Cargo.toml`); `helm template` lint on the new RuntimeClass and cert-manager resources.
- **What only a person can judge** — whether the capability set is genuinely minimal for real backend flows, and whether the gVisor audit harvest is complete enough to enforce.

## Out of scope

- The **credential-injecting egress proxy** (TLS-terminating; substitute real secrets into HTTPS headers for allowed destinations, scrub responses) — its own follow-up spec. This spec only sets the target and takes the cheap env-hygiene wins.
- The egress/ingress network model — already fail-closed.
- Broker minting internals and token TTLs beyond the peer-check.
- A VM-isolation runtime for dind repos — possible later; they fall back to the default runtime for now.
