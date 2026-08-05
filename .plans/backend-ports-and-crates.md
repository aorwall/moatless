# Make the ports singular, then split the backend into crates

> **Status:** Steps 1-7 **shipped** on `main` (1-4 as #302; Step 5 `8cd2cd32d`, Step 6 `f43e7f356`, Step 7 `c8a2dbdb6`). Steps 8-9 **partially shipped** — four of eight placement moves (`fff12035f`, `23ad5c6d6`) and one of six ports (#340, `1bdb50374`); see the Step 8 and Step 9 outcome blocks. Steps 10-11 not started: the crate-level accounting is now re-derived by `tools/crate-census` (see the re-derivation note under Architecture and key flows).
> **Updated:** 2026-08-05
> **Vocabulary:** CONTEXT.md
> **Predecessors:** `.plans/backend-crate-split.md`, `.plans/backend-crate-hierarchy.md`
> **Related:** `.plans/remaining-adapter-breakup.md` — runs after Step 4, see Out of scope.

## Current solution

`backend/src` is 140,055 lines in one crate. Two modules are half of it: `adapters/` at 37,242 lines and `sandbox/` at 34,843. The rest is long-tailed — `auth/` 9,758, `task/` 7,263, `message/` 6,410, `loops/` 4,541, `bindings/` 3,913, then eighteen modules under 3,000.

Every count in this document is taken at `origin/main` `8fe60cd`, over git-tracked `*.rs` files only. `backend/src` also contains a README, a test PEM key and an `AGENTS.md`; those are not source and are not counted. Every figure was computed twice, independently, by two agents with different scripts and different counting rules, and agreed to the reference count.

**`main` has moved since that base and the counts drift with it.** As of this branch's fork point the same measurements give 140,598 lines rather than 140,055, and 521 `AppState` references rather than 511 — about 0.4% over twelve commits. Every `file:line` anchor in this document was re-checked against the fork point and still resolves. Re-measure before quoting a figure as current; the base is pinned so that a disagreement is explicable rather than alarming.

The crate already has ports. It has 87 traits. The problem is not their absence — it is that they are not singular, not the only way in, and not consistently owned. Every finding below is a form of that one sentence.

### The domains, and every trait in them

87 trait declarations, at the line each is declared. This is the whole port surface of the workspace; the steps below move, delete, or leave each one, and touch nothing else. Declarations, not names: five sandbox ports are declared more than once, which is the subject of Step 1, and the totals below add to 87 only because each declaration is counted where it sits.

**Persistence — 30 traits.** One per aggregate, each with exactly one `Pg*` implementor.

| Trait | Declared |
|---|---|
| `AdapterAppDao` | `backend/src/adapter_apps/dao.rs:143` |
| `AdapterConnectionDao` | `backend/src/adapters/dao.rs:11` |
| `ApiKeyDao` | `backend/src/auth/api_keys.rs:25` |
| `EventReceiptDao` | `backend/src/loops/firing/receipts.rs:8` |
| `FeedbackDao` | `backend/src/feedback/dao.rs:11` |
| `FileDao` | `backend/src/file_storage/dao.rs:13` |
| `LifecycleDao` | `backend/src/sandbox/lifecycle/orchestrator.rs:107` |
| `MessageDao` | `backend/src/message/dao.rs:179` |
| `PluginDao` | `backend/src/plugin/dao.rs:12` |
| `ProviderTokenAuthDao` | `backend/src/auth/provider_tokens.rs:50` |
| `ProviderTokenDao` | `backend/src/auth/provider_token_dao.rs:21` |
| `RepositoryDao` | `backend/src/repository/dao.rs:29` |
| `SecretAuditDao` | `backend/src/secret/audit.rs:32` |
| `SecretDao` | `backend/src/secret/dao.rs:49` |
| `SessionDao` | `backend/src/auth/sessions.rs:31` |
| `TagDao` | `backend/src/tag/dao.rs:12` |
| `TaskBindingDao` | `backend/src/bindings/dao.rs:11` |
| `TaskDao` | `backend/src/task/dao.rs:38` |
| `TaskEventDao` | `backend/src/task_event/dao.rs:40` |
| `TaskRelationDao` | `backend/src/task/relations.rs:68` |
| `TaskTurnDao` | `backend/src/task_turn/dao.rs:126` |
| `TeamDao` | `backend/src/team/dao.rs:11` |
| `UserCredentialDao` | `backend/src/auth/password/dao.rs:13` |
| `UserDao` | `backend/src/auth/users.rs:58` |
| `WorkspaceDao` | `backend/src/workspace/dao.rs:61` |
| `PairingCodeDao` | `crates/backend-core/src/pairing/mod.rs:28` |
| `ConversationDao` | `crates/t3code/src/dao.rs:179` |
| `PreviewTabDao` | `crates/t3code/src/preview_tabs.rs:111` |
| `T3TaskDao` | `crates/t3code/src/data.rs:34` |
| `T3WorkspaceDao` | `crates/t3code/src/data.rs:14` |

**Sandbox runtime — 22 declarations, 16 distinct names.** Eleven of the declarations are five ports declared more than once: four twice, and `SandboxStarter` three times.

| Trait | Declared |
|---|---|
| `SandboxObserver` | `backend/src/sandbox/access.rs:42` **and** `crates/backend-core/src/sandbox/mod.rs:140` |
| `SandboxIntentReader` | `backend/src/sandbox/access.rs:79` **and** `crates/backend-core/src/sandbox/mod.rs:164` |
| `SandboxStarter` | `backend/src/sandbox/access.rs:93` **and** `crates/backend-core/src/sandbox/mod.rs:169` **and** `backend/src/task/creation.rs:53` |
| `SandboxStopper` | `backend/src/sandbox/access.rs:108` **and** `crates/backend-core/src/sandbox/mod.rs:174` |
| `SandboxConnector` | `backend/src/sandbox/access.rs:125` **and** `crates/backend-core/src/sandbox/mod.rs:179` |
| `SandboxRuntime` | `backend/src/sandbox/lifecycle/runtime.rs:137` |
| `FleetObserver` | `backend/src/sandbox/lifecycle/reconciler.rs:86` |
| `NotifyPort` | `backend/src/sandbox/lifecycle/reconciler.rs:110` |
| `AgentStopPort` | `backend/src/sandbox/lifecycle/orchestrator.rs:151` |
| `LifecycleEventSink` | `backend/src/sandbox/lifecycle/orchestrator.rs:159` |
| `LockGuard` | `backend/src/sandbox/lifecycle/orchestrator.rs:171` |
| `LifecycleLock` | `backend/src/sandbox/lifecycle/orchestrator.rs:180` |
| `WallClock` | `backend/src/sandbox/lifecycle/orchestrator.rs:193` |
| `AgentStatusPort` | `backend/src/task/reaper.rs:619` |
| `PreflightPort` | `backend/src/task/creation.rs:74` |
| `GithubValidator` | `backend/src/sandbox/preflight.rs:108` |

**Adapter capabilities — 14 traits**, all in `backend/src/adapters/capabilities.rs`. Thirteen are `pub(crate)`; only `Adapter` is `pub`. The capability surface is already crate-private, so changing it costs nothing outside the crate.

| Trait | Declared | Supertrait |
|---|---|---|
| `Adapter` | `:66` | — |
| `AdapterIdentity` | `:86` | — (blanket `impl<T: Adapter + ?Sized>` at `:101`) |
| `AdapterUserIdentity` | `:117` | — |
| `AdapterDestinationBuilder` | `:137` | `AdapterIdentity` |
| `AdapterOutboundContent` | `:194` | `AdapterIdentity` |
| `AdapterBindingReply` | `:208` | `AdapterOutboundContent` |
| `AdapterMessageReply` | `:238` | `AdapterOutboundContent` |
| `AdapterDirectDestination` | `:252` | `AdapterOutboundContent` |
| `AdapterDirectMessage` | `:263` | `AdapterIdentity` |
| `AdapterPostBindingFollowup` | `:270` | `AdapterIdentity` |
| `AdapterNewThread` | `:283` | `AdapterOutboundContent` |
| `AdapterReactions` | `:294` | `AdapterIdentity` |
| `AdapterInboundPresentation` | `:312` | — |
| `AdapterBindingStatus` | `:360` | — |

**Delivery and events — 4.** `AdapterDeliveryHandler` `backend/src/adapters/delivery/handlers.rs:20`, `ConnectionDeliveryPayload` `backend/src/adapters/delivery/typed.rs:23`, `EventPublisher` `backend/src/events.rs:200`, `IngressOutbox` `backend/src/event_ingress/outbox.rs:54`.

**Auth and secrets — 3.** `OAuthProvider` `backend/src/auth/oauth/provider.rs:9`, `SecretCipher` `backend/src/secret/service.rs:30`, `PairingApiKeyIssuer` `crates/backend-core/src/pairing/mod.rs:77`.

**Platform — 4.** `LeaderGuard` `backend/src/leader.rs:13`, `SkillFetcher` `backend/src/skill/fetcher.rs:63`, `LoopFetcher` `backend/src/loops/sync/fetcher.rs:35`, `SetupRequirementCheck` `backend/src/setup/evaluator.rs:26`.

**T3 compatibility — 4.** `AgentCatalogSource` `crates/t3code/src/ports.rs:107`, `RepositoryBranchSource` `:136`, `ThreadCommandSink` `:146`, `T3ActivityViewExt` `crates/t3code/src/activity.rs:13`.

**Vendored SDK internals — 6, out of scope.** `Transport` and `StdinWriter` `crates/claude-agent-sdk/src/transport/mod.rs:14,:23`, `HookHandler` `crates/claude-agent-sdk/src/types/hooks.rs:53`, `PermissionHandler` `crates/claude-agent-sdk/src/types/permissions.rs:55`, `ApprovalHandler` `crates/codex-sdk/src/approvals.rs:15`, `HookObserver` `crates/codex-sdk/src/hooks.rs:9`. These are the SDKs' own abstractions over their subprocess protocols, not the backend's ports.

### What research turned up that the code does not say

**Five sandbox ports are declared twice, method-for-method identical.** `backend/src/sandbox/access.rs` and `crates/backend-core/src/sandbox/mod.rs` each declare `SandboxObserver`, `SandboxIntentReader`, `SandboxStarter`, `SandboxStopper`, `SandboxConnector` with the same method names and the same argument lists. They differ only in error type — the local set returns `backend::sandbox::error::SandboxError`, the core set returns `backend_core::sandbox::SandboxError`.

The consequence is visible at the composition root. `backend/src/main.rs:147-151` declares five `Arc<dyn>` slots:

```rust
let sandbox_runtime: Arc<dyn sandbox::lifecycle::SandboxRuntime>;
let sandbox_observer: Arc<dyn sandbox::access::SandboxObserver>;
let sandbox_connector: Arc<dyn sandbox::access::SandboxConnector>;
let ui_rpc_sandbox_observer: Arc<dyn moatless_backend_core::sandbox::SandboxObserver>;
let ui_rpc_sandbox_connector: Arc<dyn moatless_backend_core::sandbox::SandboxConnector>;
```

and `main.rs:174-178` fills all five by cloning **one** `adapter`. The same object is held behind two names for the same capability, because the port was copied into `backend-core` rather than moved there.

**`backend/src/t3code/backend.rs` is 685 lines that exist mostly to bridge that copy.** It implements the `backend-core` copies of all five ports for the same concrete types — `DockerSandboxAdapter`, `K8sSandboxAdapter`, `Lifecycle` — that already implement the `backend/src` copies in `sandbox/docker/provider.rs:583,:627`, `sandbox/k8s/provider.rs:684,:744`, and `sandbox/access.rs:84,:98,:113`. Every bridging method body has the same shape:

```rust
backend_sandbox::SandboxConnector::sandbox_client(self, task_id).await.map_err(sandbox_error)
```

**A third `SandboxStarter` exists with a degraded error type.** `backend/src/task/creation.rs:53` declares `async fn start(&self, task_id: &str) -> Result<(), String>` — same name, same method, same implementor (`crate::sandbox::lifecycle::Lifecycle`, at `:60`), errors stringified.

**The two `SandboxError` types are 22 variants and 7.** `backend/src/sandbox/error.rs` carries 22 variants including `KubeError(#[from] kube::Error)` at `:29` and a `bollard::errors::Error` source at `:46` — it drags both container runtimes into the type. `crates/backend-core/src/sandbox/mod.rs:12` carries 7 transport-shaped variants plus `classify_grpc_error` at `:35`. Conversion glue runs both directions: `backend/src/sandbox/client.rs:61-79` and `backend/src/t3code/backend.rs:125-141`.

**The DAO ports are not the only way to the database.** `TaskBindingDao` and `AdapterConnectionDao` are declared as ports, and the concrete `PgTaskBindingDao(pub DbPool)` (`bindings/dao.rs:100`) and `PgAdapterConnectionDao(pub DbPool)` (`adapters/dao.rs:42`) are tuple structs over the pool — so any code holding `state.db` constructs one in a single expression. The same public-tuple-field shape recurs on other DAOs: `PgEventReceiptDao(pub DbPool)` (`loops/firing/receipts.rs:5`) and the five DAOs `ProvisionDeps::from_pool` builds (`sandbox/lifecycle/plan.rs:34`). **46 sites across 23 files do exactly that**, outside the two legitimate constructions in `state.rs:358-359`:

| File | Sites |
|---|---|
| `backend/src/sandbox/lifecycle/plan.rs` | 5 — `:36,:37,:38,:41,:42` |
| `backend/src/loops/sync/mod.rs` | 5 — `:305,:337,:347,:391,:401` |
| `backend/src/bindings/runtime.rs` | 5 — `:86,:165,:403,:683,:786` |
| `backend/src/bindings/outbound.rs` | 3 — `:49,:234,:249` |
| `backend/src/adapters/providers/linear/sessions.rs` | 3 — `:137,:235,:262` |
| `backend/src/adapters/providers/github_pr/routing.rs` | 3 — `:118,:352,:644` |
| `backend/src/bindings/reply.rs` | 2 — `:38,:212` |
| `backend/src/adapters/providers/linear/subject.rs` | 2 — `:109,:137` |
| `backend/src/adapters/providers/linear/issues.rs` | 2 — `:210,:397` |
| `backend/src/adapters/providers/github_pr/merge_conflict.rs` | 2 — `:76,:175` |
| `backend/src/adapters/common.rs` | 2 — `:194,:255` |
| `backend/src/task/reaper.rs` | 1 — `:958` |
| `backend/src/loops/firing/listen.rs` | 1 — `:200` |
| `backend/src/bindings/status_service.rs` | 1 — `:99` |
| `backend/src/auth/routes.rs` | 1 — `:191` |
| `backend/src/adapters/providers/linear/close.rs` | 1 — `:43` |
| `backend/src/adapters/providers/linear/auth.rs` | 1 — `:112` |
| `backend/src/adapters/providers/github_pr/status.rs` | 1 — `:30` |
| `backend/src/adapters/providers/github_pr/post_merge.rs` | 1 — `:191` |
| `backend/src/adapters/providers/github_pr/ingress.rs` | 1 — `:335` |
| `backend/src/adapters/providers/github_pr/delivery.rs` | 1 — `:158` |
| `backend/src/adapters/inbound_delivery.rs` | 1 — `:165` |
| `backend/src/adapters/delivery/typed.rs` | 1 — `:105` |

Four function signatures also name the concrete type rather than the port: `linear/close.rs:60,:84,:140,:233` all take `binding_repo: &PgTaskBindingDao`.

Separately, 35 files under `backend/src` outside a `dao.rs` contain a raw `sqlx::query`.

**There are two composition roots and neither is authoritative.** `backend/src/state.rs` mentions `Pg*Dao` types 23 times; `backend/src/startup.rs` mentions them 15 times and independently builds its own trait objects from the same pool. `auth/routes.rs:191` builds a third set. A reader asking "where does a `DbPool` become a port" has three answers.

**`AppState` is 40 fields**, all `pub`, at `backend/src/state.rs:58`. The name occurs 511 times under `backend/src` after comments are stripped, across 26 of the 46 module-level nodes and 23 of the 29 directories: 214 as `&AppState`, 138 as `State<AppState>`, 99 as an import, 30 in declarations and signatures. Because every field is public and the struct is passed whole, holding an `AppState` is equivalent to holding every port plus the raw pool — which is why the 46 bypasses above are so easy to write.

**`BackendSandboxServiceImpl` is the same shape, built a second time.** `backend/src/sandbox/grpc_server.rs:105-176`, 26 fields, **20 of them `Option`**, assembled at `startup.rs:517`. Eighteen field names duplicate `AppState` field names. It names `AppState` exactly once, in a doc comment at `:1439` — so it needs nothing from `AppState`. It was already decomposed away from it, then grew back into a second god object. It also carries `backend_config: Option<Arc<Config>>` at `:152` — the whole `Config`, of which it reads one field directly (`sandbox.user_token_ttl_secs`) and forwards the rest wholesale to two prompt builders.

The `Option` is not optionality in the domain. It encodes deployment mode: whether the process was started with a database. The same pattern appears at `backend/src/adapters/registry/bootstrap.rs:20`, which returns `Option<Self>` documented "Returns None when no providers are configured" — but the five `register_*` calls at `:53,:58,:67,:80,:92` are unconditional and `:95` always returns `Some(providers)`. The `None` arm is unreachable.

**The capability registry has 14 traits, 13 `HashMap<AdapterKind, ...>` maps, and 18 registrars** (`backend/src/adapters/registry/mod.rs`). Two of the maps are provably unnecessary:

- `AdapterIdentity` has a blanket `impl<T: Adapter + ?Sized>` at `capabilities.rs:101` whose methods forward to `Adapter`'s. Every `Adapter` is an `AdapterIdentity` automatically, so its map can never hold a different key set than the `Adapter` map. Every trait declared `: AdapterIdentity` is therefore effectively declared `: Adapter`.
- `AdapterUserIdentity` has **zero production dispatch**. It has five implementors (`github_pr/auth.rs:267`, `linear/adapter.rs:137`, `slack/auth.rs:203`, `telegram/auth.rs:39`, `webhook/adapter.rs:215`), a map at `registry/mod.rs:35`, five registrar calls at `:165,:177,:191,:205,:224`, an accessor at `:282`, and an error constructor at `:487`. The only callers of that accessor are two test assertions, `registry/tests.rs:89` and `:107`. The code already says so: `capabilities.rs:120-128` carries a `dead_code` allow. `sender_label_suffix` at `:331` carries its own `expect(dead_code)`.

**The registry names concrete adapter types.** `backend/src/adapters/registry/mod.rs:46-49` holds `Option<Arc<super::github_pr::adapter::GithubPrAdapter>>` and the equivalent for the other four providers, and `delivery/mod.rs:139-143` calls `<provider>::delivery::register(handlers, state)` per provider. This is what makes the providers inseparable from the registry today.

**Two `ActivityKind` enums model the same concept differently.** `crates/backend-core/src/activity/mapping.rs:103` has nine variants (`ToolCall`, `Thinking`, `SandboxStatus`, `SandboxWarning`, `SandboxError`, `UserInputRequested`, `UserInputResolved`, `PlanUpdated`, `ContextWindowUpdated`). `backend/src/adapters/providers/linear/agent_api.rs:55` has five (`Thought`, `Action`, `Response`, `Elicitation`, `Error`). The second is Linear's wire vocabulary and correctly belongs to Linear; the name collision is the defect.

**`crates/t3code` is already the shape this program is trying to reach, and nothing in the repository says so.** Its `Cargo.toml` names `moatless-api-schemas`, `moatless-backend-core` and `sandbox-proto` — and no backend crate. It declares nine of its own ports (`T3WorkspaceDao`, `ConversationDao`, `SandboxConnector`, `SandboxObserver`, `ClusterDiagnostics`, `T3TaskDao`, `SandboxIntentReader`, `SandboxStarter`, `SandboxStopper`), takes them in one `UiRpcDependencies` struct at `lib.rs:392`, and reaches into no god object. 19,696 lines on the far side of a port boundary that already holds. Every pattern Steps 5, 9 and 10 propose has a working instance here, and the reason it is worth naming is that the program otherwise reads as inventing a shape rather than finishing one.

**The surface is off by default and is heading to default-on.** `lib.rs:527`'s `is_enabled()` reads `T3_UI_RPC_ENABLED` and falls back to `false`; `router_from_env` (`:308-313`) returns `Ok(None)` when unset, so `main.rs:482`'s mount is skipped. That flip is a product decision outside this program, but it changes two things this program touches. First, `router_from_env` returns `Result<Option<Router>, UiRpcConfigError>` and `task_listing/dao.rs:76` records the deliberate choice that it *"fails startup rather than inventing an identity"* — so a deployment with incomplete T3 configuration boots today only because the router is never built, and stops booting when the default flips. Second, `auth/middleware.rs:84`'s unauthenticated `/oauth/token` path stops being a 404 anywhere, which makes device pairing and API-key issuance a live surface on every deployment rather than an opted-in one.

**The test tier does not build what it claims.** `.github/workflows/ci.yml:161` runs `cargo test -p moatless-vibe-backend` with no `--features db-tests`, so every `#[sqlx::test]` is compiled out of the run. The repository's real integration entry point is `task test:integration` (`Taskfile.yml:340`).

### The module graph, measured

`backend/src` is 46 module-level nodes with 1,455 `crate::` references over 305 ordered pairs. **One strongly connected component contains 29 of the 46**, held shut by 45 mutually-referencing pairs — `sandbox`↔`task` at 57/18, `adapters`↔`bindings` at 41/32, `adapters`↔`auth` at 34/12, `adapters`↔`state` at 40/4, `auth`↔`sandbox` at 35/2.

The node count, the 305 pairs, the component and all five pair counts were reproduced independently. The `1,455` total was not, and the two counting defects described under **The target** — dropped root-file references and unresolved `super::` paths — bear on it, so it is re-derived with the crate-level numbers rather than carried forward.

Simulating Steps 1–6 against that graph leaves a 26-module component, which is membership-identical to simply deleting the `state` node: only `feedback` and `tag` leave the cycle, and none of the 28 non-`state` mutual pairs is broken. **Steps 1–6 are worth exactly "remove the god objects" — no more.** That is not an argument against them; removing the god objects is the prerequisite for everything else, and the ports work is what makes the remaining edges legible. But `AppState` is not the tangle, and a reader who finishes Step 6 expecting a layered backend will not have one. Steps 8–11 are where the layering happens.

### What is already measured

The build-time argument for splitting is real and was measured on this checkout: an edit inside the runtime half goes from 17.9s to 1.72s; an edit inside the domain half goes from 18.0s to 15.4s. The second number is the one that matters — splitting by direction alone barely moves the common case, because the domain half stays large. Architecture, not build time, is what justifies the program below; the build win is a consequence.

The prior spike is on branch `split-backend-by-direction` at `9fdfa0e`, explicitly non-compiling, with `HEAD~1` the last green commit. It is evidence about build times, not a starting point.

The sandbox is provisioned at `requests: {cpu 2, memory 4Gi}`, `limits: {cpu 4, memory 8Gi}` with `CARGO_BUILD_JOBS: "2"` (`.moatless/workspaces.json`). Pods exceeding their memory *request* are evicted first under node pressure, so the request is the number that governs stability, not the limit.

## Planned solution

**New**

- One sandbox runtime contract owning the five sandbox ports and one `SandboxError`, declared once in the workspace.
- One composition root — a single function that is the only place a `Pg*Dao` is constructed.
- A CI check that fails when a new `DbPool`-to-DAO construction appears outside that root.
- Fifteen crates under `crates/`, replacing the single `backend/` crate: nine bands, the adapter vocabulary, two sandbox providers, five adapter providers minus the overlap, and a root that is only wiring.
- Six consumer-owned ports that do not exist today, each declared by the crate that calls upward — listed in Step 9.
- Two more, declared by `bindings` and implemented by the adapter registry: `OutboundProviders` and `BindingStatusSource`, in Step 7c. (That these are two ports and not one is the crate-split side's glimpse of the several-jobs breadth recorded at the "`bindings` reads twelve ports" note under Step 5 — whether `bindings` becomes one crate or several is settled there, not here.)

**Changes materially**

- `backend/src/sandbox/access.rs:42-125` — the five port declarations are removed; the file keeps `LogOptions` and its documentation role.
- `crates/backend-core/src/sandbox/mod.rs:12-200` — becomes the sole declaration site, or yields it to a new crate.
- `backend/src/t3code/backend.rs` — the bridging implementations are deleted; the `AgentCatalogSource` and `ThreadCommandSink` implementations remain, and the DTO projections are rehomed rather than deleted.
- `backend/src/sandbox/error.rs` — the port error stops naming `kube` and `bollard`.
- `backend/src/main.rs:147-178` — the five duplicated `Arc<dyn>` slots collapse. `cluster_diagnostics` is declared in the same range and is a sixth port that does not dedup, so the range keeps a distinct slot for it.
- `backend/src/state.rs:58` — `AppState` stops being a type any handler names.
- `backend/src/startup.rs` — stops being a second composition root.
- `backend/src/sandbox/grpc_server.rs:105-176` — 26 fields become the dependencies actually used, without `Option`.
- `backend/src/adapters/registry/mod.rs` — 13 maps become 11, 18 registrars shrink accordingly, and the five `Option<Arc<ConcreteAdapter>>` fields at `:46-49` become `Arc<dyn>`.
- `backend/src/adapters/capabilities.rs` — `AdapterUserIdentity` is deleted, `AdapterIdentity`'s registry entry is deleted, `sender_label_suffix` is deleted.
- `backend/src/task/connections_handlers.rs` and `task/handlers.rs:22-23,248,1556` — binding code filed under `task/` moves to the loops crate.
- `backend/src/access_control.rs`, `credentials/{github,codex}/scope.rs`, `adapters/providers/github_pr/api.rs`'s `api_base`, `provider_settings/{error,schemas,github_client}.rs`, `task_turn::models::UnclosedTurn` — each moves to the crate its callers are already below.
- The 46 bypass sites and the 4 concrete-type signatures listed above.
- `backend/Cargo.toml` — the package stops being `moatless-vibe-backend`; the root is `crates/server`, package `moatless-server`.
- `sandbox/Cargo.toml` — the package becomes `moatless-sandbox-agent`, with the 26 `-p` flags, path dependencies and `Cargo.lock` entries that name it.
- `AGENTS.md` and five workflows — the `-p` flags in the clippy and test blocks, rewritten in the same commit as each rename.
- `backend/src/adapters/models.rs` and `adapters/dao.rs` — move intact into `moatless-adapter-model`. `adapters/error.rs` and `adapters/common.rs` split, the port-facing half going down and the cross-domain conversions staying.
- `backend/src/adapters/capabilities.rs:230-231` — stops naming `crate::bindings`; the two conversions it calls move down beside the types they convert.
- `backend/src/bindings/outbound.rs` and `bindings/direct_send.rs` — take a port instead of `&Adapters`. The other ten `bindings` files stop naming `adapters` entirely.
- `backend/src/adapters/registry/mod.rs:240,:245` — `linear()` and `slack()` are deleted; `providers/linear/handlers.rs:33-40`, `providers/slack/handlers.rs:26-33` and `providers/telegram/poller.rs:12,80` receive their own adapter instead of fetching it back out of the registry.
- `backend/src/adapters/routes.rs:9,28-72` — the `bindings::handlers::*` mounts move to the server crate.
- `.github/workflows/ci.yml:161` — the database tier actually runs.

**Untouched**

- Every HTTP route, request shape and response shape. No API contract changes.
- Every database table, column and index. No migrations.
- All adapter behaviour — Slack, Telegram, Linear, GitHub PR, webhook. Inbound normalization, outbound delivery, reactions and binding status all behave identically.
- `apps/frontend` and the generated `@moatless/api-client`.
- `crates/claude-agent-sdk` and `crates/codex-sdk`, including their six internal traits.
- The T3 client protocol — `crates/t3code`'s public surface and the UI-RPC methods it serves. Its nine ports and `UiRpcDependencies` are the model this program follows, not something it edits, and whether `T3_UI_RPC_ENABLED` defaults on is decided elsewhere.
- Sandbox provisioning, images, and the gRPC wire protocol between backend and sandbox.
- Every Docker image name, Kubernetes pod name, Service name and label. The `moatless-sandbox` *package* is renamed; the 182 places that string names a deployment artifact are not.
- `reconcile_shell_command` and the rest of `github_pr`'s 1,848 lines of behaviour; it is GitHub semantics, not a layering artifact, and does not move.
- `backend/src/sandbox/error.rs`'s 22 variants and its axum `IntoResponse`. It stays as the sandbox module's HTTP error; only what the *ports* return becomes singular.

### Problem

Three people feel this, differently.

**The person changing adapter behaviour** waits 18 seconds for a check on a one-line edit, and cannot tell from the type system which parts of the system their change can reach. Holding an `AppState` means holding the pool, so any handler can query any table. Reviewing a change means reading it, not trusting a boundary.

**The person adding a sandbox provider** has to implement five ports twice — once in `backend/src` and once in `backend-core` — and discovers this only after the first set compiles. Nothing in the code says the second set exists; `t3code/backend.rs` is where they find out.

**The person adding an adapter** implements against 14 capability traits, one of which is never dispatched in production and one of which is satisfied automatically by a blanket impl, and both of which still require registration. The registry's 18 registrars do not distinguish the traits that matter from the two that do not.

Underneath all three: the ports exist but are decoration. A port that is not the only way in constrains nothing, and a port declared twice constrains less than one declared once.

### Behaviour

This program changes no observable behaviour. The acceptance statements are therefore about what stays true and what becomes impossible.

**As an operator**

- **Given** a deployment on any supported sandbox provider, **when** the program completes, **then** every API response, event payload, and adapter delivery is identical to before.
- **Given** a running cluster, **when** the two package renames land, **then** no pod, image, Service or label name changes and no redeploy is required for them.
- **Given** a process started without a database, **when** it starts, **then** it fails with a clear error at startup rather than carrying 20 `Option` fields that are `None` — or, if databaseless operation is a supported mode, that mode is named as its own type and its supported surface is enumerated. Step 6 settles which; it is currently neither.

**As a developer changing domain code**

- **Given** a change to adapter or binding logic, **when** they need data, **then** the only reachable way to get it is a port their own signature names.
- **Given** an attempt to construct `PgTaskBindingDao(pool)` outside the composition root, **when** CI runs, **then** the build fails naming the file and line.
- **Given** a change in a low band that reaches into a higher one, **when** they build, **then** it does not link — the dependency is not expressible rather than discouraged.

**As a developer adding a sandbox provider**

- **Given** the five runtime ports, **when** they implement each once, **then** the provider is complete — there is no second set, and no bridging module to write.
- **Given** a new provider crate, **when** it is added, **then** nothing but the composition root names it, and removing it is deleting a directory and a line of wiring.

**As a developer adding an adapter**

- **Given** an adapter that cannot perform a capability, **when** a caller requests it, **then** the registry returns the typed `AdapterError` for that capability and the caller's behaviour is unchanged from today. This is asserted by test before any map is deleted — see Step 4.

### Architecture and key flows

**Eleven steps, in three parts.** Each step leaves the tree green and is separately shippable. Steps are numbered by execution order; where two are independent, that is stated, and either order works.

| Part | Steps | What it achieves |
|---|---|---|
| **I — Make the ports singular** | 1–4 | Every port is declared once and is the only way in. Ends with the module graph unchanged in shape. |
| **II — Remove the god objects** | 5–7 | No type means "everything the process has", and the largest module cycle is broken rather than absorbed. Ends with a tree that is ready to be layered. |
| **III — Split into crates** | 8–11 | The bands become packages, so direction is checked by Cargo rather than by review. |

Dependencies: 2 → 3 → 5; 5 → 10; 3 → 7; 7d → 10h. Steps 1, 4 and 6 are independent of everything except that 6 wants 2. Steps 8 and 9 want the tree quiet, so they follow Part II, but neither depends on it for correctness.

**Where it is safe to stop.** Programs like this stop early, so the resting points are specified rather than left to whoever runs out of time.

| Stop after | State | Coherent? |
|---|---|---|
| Step 4 | Ports singular, registry consistent. `AppState` intact, 29-module SCC intact | Yes — a real improvement that stands alone |
| Step 6 | God objects gone. 26-module SCC remains, unnamed | **Marginal.** Prefer stopping at 7 — The gain over Step 4 is "remove the god object", which is worth having, but a 26-module component nobody has named is worse to inherit than the 29 it replaced, because the tree now *looks* decomposed |
| Step 7 | God objects gone, `adapter-model` extracted, `adapters`↔`bindings` broken | Yes — the first point where a direction is enforced by Cargo rather than by review, and the resting point to prefer over Step 6 |
| Step 9 | Placement fixed, five ports inverted. Still one crate; band order holds by convention only | **No.** Convention decays. Everything from here is throwaway unless Step 10 lands |
| Step 10 | Fifteen crates, direction enforced by Cargo | Yes — this is the program's real endpoint |
| Step 11 | Names consistent | Yes |

**Do not stop between Step 8 and Step 10.** Steps 8 and 9 are the cost of Step 10 with none of its benefit.

---

#### Step 1 — Collapse the duplicated sandbox ports

*Independent of every other step. Highest deletion-to-risk ratio in the program, which is why it goes first.*

- **Observed** — five ports declared twice (`sandbox/access.rs:42,:79,:93,:108,:125` and `backend-core/src/sandbox/mod.rs:140,:164,:169,:174,:179`), a third `SandboxStarter` at `task/creation.rs:53` returning `Result<(), String>`, 685 lines at `t3code/backend.rs` of which the bridging impls are the bulk, two `SandboxError` types of 22 and 7 variants, conversion glue at `sandbox/client.rs:61-79` and `t3code/backend.rs:125-141`.
- **Proposed responsibility** — one module answers "what can a caller ask of a sandbox runtime, and how does it fail".
- **Proposed change** — the five ports and one `SandboxError` live in a single place that both `backend` and `t3code` depend on. `sandbox/access.rs` keeps `LogOptions` and its doc header. `task/creation.rs`'s `SandboxStarter` is deleted and its caller takes the real one. The bridging impls in `t3code/backend.rs` are deleted; its `AgentCatalogSource` and `ThreadCommandSink` impls stay. `main.rs`'s five `Arc<dyn>` slots become three.
- **Evidence** — `main.rs:174-178` clones one `adapter` into all five slots, so no call site distinguishes the two families today.

**The connector dedups; the observer does not.** `SandboxConnector`, `SandboxStarter`, `SandboxStopper` and `SandboxIntentReader` are error-only collapses, so one trait object serves both surfaces and the two slots become one. `SandboxObserver` cannot: the DTO section below keeps the thirteen-field read backend-local and gives T3 the two-field contract, so the contract observer is **projected from** the rich reader through a wrapper rather than being the same object under a second name. Counting the observer as a dedup is what makes "five slots become three" read as a smaller change than it is.

**Commits.**

| # | Commit | Gate |
|---|---|---|
| 1 | Provider-classify the error: `ProviderKind {Kubernetes, Docker}` and `Provider {kind, message}` in `backend-core`; remove the wildcard arms in `t3code/backend.rs`'s `sandbox_error` and `sandbox/client.rs`'s `classify_grpc_error` | Four clippy lines + unit tests. Nothing observable changes |
| 2 | Rehome the DTO projections out of the port impls | Docker E2E profile |
| 3 | Delete the `backend-core` port declarations' duplicates; one declaration each | Docker E2E profile |
| 4 | Delete `task/creation.rs:53`'s third `SandboxStarter`; caller takes the real port | Docker E2E profile |
| 5 | Collapse the five duplicated slots at `main.rs:174-178`. `cluster_diagnostics`, declared in the same range, is a sixth port with one declaration already and keeps its own slot | Docker E2E profile |

The unified port error must not name `kube` or `bollard` in its public shape. `backend/src/sandbox/error.rs:29` has `KubeError(#[from] kube::Error)` and `:46` a `bollard::errors::Error` source; both become provider-classified variants carrying a message and a kind, with the provider crates' errors converted at the provider boundary. `backend/src/sandbox/error.rs:337`'s test moves with the type. This is what lets a caller depend on the contract without compiling either container runtime — the whole point of the port, and the precondition for `sandbox-k8s` and `sandbox-docker` being separate crates in Step 10.

**"One `SandboxError`" means one *contract* error.** `backend/src/sandbox/error.rs` declares 22 variants and an axum `IntoResponse`; it is the sandbox module's HTTP error as much as a port error, and its `Conflict` and `Forbidden` arms are how routes produce 409 and 403. Collapsing that enum is an API change and is not in this step. What becomes singular is the error the five ports return — `backend-core`'s, kube- and bollard-free — with the module error converted to it at the port boundary, arm by arm. The conversion that exists today at `t3code/backend.rs:242-259` ends in an `other => Other(to_string())` tail that flattens most of the enum into a string; the replacement enumerates every variant, and **adding a variant without deciding its contract mapping must fail to compile.**

**The value types are not duplication, and are not deleted with the glue.** `t3code/backend.rs:183-240` is not error mapping. It projects `SandboxStatusFullResponse`, `LiveStatus`, `ServerEntry`, `ServerStatus`, `ServerCapabilities` and `LogOptions` from the backend's vocabulary into a deliberately smaller one. The backend's status type (`sandbox/schemas.rs:134-172`) carries thirteen fields — `sandbox_error`, `agent_status`, `resources`, diagnostics, runtime events, jobs, background processes, desired state and the three image fields; the core type (`backend-core/src/sandbox/mod.rs:115-118`) carries two. `sandbox_status()` at `:183` discards everything but `sandbox_status` and `servers`.

The projection is **one method**. `SandboxObserver::live_status` returns `backend/src/sandbox/live_status.rs:39` and `backend-core/src/sandbox/mod.rs:131`, which are field-identical (`sandbox_status`, `agent_status`, `sandbox_error`); `server_logs` returns a `BoxStream<Result<String, _>>` on both sides. Those two, and all four of the other ports, are error-only collapses. Only `sandbox_status` differs in value type, and it differs because `backend/src/sandbox/handlers.rs:109` serves the thirteen-field type as a documented HTTP body. That route keeps its shape: the rich assembly stays backend-local under a name of its own, and the contract `SandboxObserver` is implemented by projecting from it. Pushing the rich type down into `backend-core` instead would drag `ResourceMetrics`, `ContainerDiagnostics`, `RuntimeEvent` and `ScheduledJobInfo` with it, which is the junk drawer this step exists to prevent.

That narrowing is a feature: it is the T3-facing contract, and widening it by unifying the types would hand a client a surface nobody chose to give it. So Step 1 unifies **the five ports and the error, and keeps the DTO projection** — but moves it out of `t3code/backend.rs`'s port impls and names it for what it is: an outbound projection to the T3 contract, sitting beside that contract rather than hidden inside a trait implementation. A reviewer who reads "delete the bridging impls" and expects all 685 lines to disappear will be surprised, and an implementer who deletes the projections will silently widen the T3 contract.

Whether the contract lives in a new crate or in `backend-core`'s existing `sandbox` module promoted is an implementation choice, not a spec commitment. The requirement is that exactly one declaration of each port exists in the workspace, and that `backend-core` holds contract vocabulary only — no pool, no config, no process policy. It has been a junk drawer before; the constraint is what keeps it from becoming one again with a better name.

#### Step 2 — One composition root

*Precondition for Steps 3, 5 and 6.*

- **Observed** — `state.rs` mentions `Pg*Dao` 23 times, `startup.rs` 15 times and builds its own trait objects from the same pool, `auth/routes.rs:191` builds a third set.
- **Proposed responsibility** — one function answers "given process configuration, what ports exist". It is the only place a `Pg*Dao` is constructed.
- **Proposed change** — a single `compose` entry point returning the assembled ports. `startup.rs` and `auth/routes.rs` receive built ports instead of building them.
- **Evidence** — the three sites above are the complete set of places trait objects are constructed today; every one of the 46 sites in Step 3 constructs a concrete `Pg*Dao` for immediate single use instead.

**The invariant is about DAO construction, not about `DbPool` visibility.** Those are different properties and only the first is achievable. Plenty of code legitimately holds a pool for reasons that are not "build a DAO": `state.rs:61`, `adapter_apps/mod.rs:125`, `adapters/delivery/store.rs:35-52`, `adapters/providers/linear/adapter.rs:38-75`, `auth/routes.rs:117-154`, `sandbox/lifecycle/lock.rs:27-33`, and `startup.rs:268,346,420,617`. Those are raw-pool services and test seams, and they are the same population as the 35 raw-`sqlx::query` files that Step 3 explicitly does not touch. A "no `DbPool` outside the root" rule would either fail on day one or force those services into a shape nobody designed.

So the enforceable property, and the only one either step claims, is: **no `Pg*Dao` is constructed outside the composition root.** Raw-pool services are a separate, later question — see Out of scope.

This step moves no logic, with one exception it should fix while it is here. **The third composition root is duplicating a cache, not just constructing ports twice.** `AuthState::new` (`auth/routes.rs:164-171`, built at `main.rs:361`) constructs its own `GitHubAppRegistry` with its own installation-token cache, while `main.rs:387-395` builds the registry the code describes as built once and shared. The consequence is doubled installation-token minting against GitHub's quota and the round-trips that go with it. It is **not** a stale-token correctness bug — the per-lookup `record_id` + revision check evicts both caches — but `carry_over_env_apps` runs on only one of them. Collapsing to one root fixes it for free, which is why it is named here rather than filed as its own defect.

The database test tier is already covered, so this step inherits it rather than fixing it. The backend's clippy line carries `--features db-tests`, which type-checks the 39 `#![cfg(feature = "db-tests")]` files in `backend/tests`, and `ci.yml` additionally links them with `cargo test -p moatless-vibe-backend --features db-tests --no-run`. That matters to every later step, because each one cites database-backed tests as its behavioural evidence.

#### Step 3 — Close the 46 bypasses

*Requires Step 2.*

- **Observed** — 46 construction sites in 23 files (table above), plus 4 signatures naming `&PgTaskBindingDao` at `linear/close.rs:60,:84,:140,:233`.
- **Proposed responsibility** — the port is the only way to the table.
- **Proposed change** — each site takes the port it needs instead of constructing from `state.db`. Sites needing a method the port does not expose get the method promoted onto the trait rather than keeping the concrete type; `find_by_scope_value` and `list_by_provider` on `AdapterConnectionDao` are the two known cases. `linear/close.rs` takes `&dyn TaskBindingDao` in all four signatures. `PgTaskBindingDao`'s and `PgAdapterConnectionDao`'s tuple fields become private.
- **Evidence** — `PgTaskBindingDao(pub DbPool)` at `bindings/dao.rs:100` and `PgAdapterConnectionDao(pub DbPool)` at `adapters/dao.rs:42` are tuple structs with public fields, which is what makes a bypass a single expression. Making the field private is the mechanical half of this step and the part that prevents regression.

The 35 files with raw `sqlx::query` outside a `dao.rs` are **not** in this step. They are a mix of genuine one-off queries, migration tooling (`secret/cli_migrate.rs`), and test support (`sandbox/lifecycle/test_support.rs`, `contract_tests.rs`, `telegram/webhook_tests.rs`). Classifying them is its own piece of work and is listed under Out of scope; conflating it with the 46 mechanical sites is what would turn a week into a quarter.

The CI check added here greps for the `Pg[A-Za-z]+Dao(` constructor outside the composition root and the `dao.rs` files, and fails with the offending `file:line`. A grep is the right instrument because the property is syntactic: the tuple-struct constructor is the exact thing being banned, and it is greppable precisely because it is a single expression.

#### Step 4 — The capability registry consistency pass

*Independent of every other step.*

- **Observed** — 14 traits, 13 maps, 18 registrars. `AdapterIdentity` blanket-impl'd at `capabilities.rs:101`; `AdapterUserIdentity` dispatched only from `registry/tests.rs:89,:107`, with a `dead_code` allow at `capabilities.rs:120-128` recording the fact. Four `Option<Arc<ConcreteAdapter>>` fields at `registry/mod.rs:46-49` (github_pr, linear, slack, telegram; webhook has no typed handle).
- **Proposed responsibility** — a map exists for a capability exactly when an adapter can meaningfully lack it, and the registry names no concrete provider. The map half lands here; the concrete-provider naming is erased in Step 7d, which deletes the typed accessors that force it.
- **Proposed change** — three deletions and one relocation land here; a fourth change, the field widening, is scheduled into Step 7d:
  1. `AdapterUserIdentity`: the trait, its five implementors, its map (`registry/mod.rs:35`), its five registrar calls (`:165,:177,:191,:205,:224`), its accessor (`:282`), its error constructor (`:487`), and the two test assertions that are its only callers.
  2. `AdapterIdentity`'s map and registrar. **This one is a rewrite, not a drop.** `identity_by_name` (`registry/mod.rs:431`) and `adapter_kind_for_name` (`:436`) both resolve names by iterating `self.identities`, so the map has a live reader even though it is structurally redundant. Both are re-expressed against the `adapters` map. The blanket impl at `capabilities.rs:101-113` forwards all three methods — `adapter_kind`, `provider_key`, `connection_provider_key` — to `Adapter`, so every method those two functions call is available on the `Adapter` objects already in that map. The rewrite is mechanical, but it is a behavioural surface and gets its own test.
  3. `sender_label_suffix` (`capabilities.rs:331`), overridden only by Slack at `slack/context.rs:18` and asserted only by that file's tests at `:226,:230,:234`.
  4. `delivery/mod.rs:139-143`'s per-provider `register` calls move to the composition root, so the delivery layer no longer names the five concrete providers by path. This is the Step 4 half — a registry-adjacent relocation, testable today.

  The other half — widening `registry/mod.rs:43-46`'s four `Option<Arc<ConcreteAdapter>>` fields to `Arc<dyn>` — **belongs to Step 7d, not here.** Widening a field while its accessor still returns a concrete type only buys an `as_any` and a downcast: the field and its accessor are a single change, and 7d is where the accessors are deleted and their callers migrated to router state. **That is the edge that makes the five adapter crates possible in Step 10.**

  Result in Step 4: 12 traits, 11 maps, and the delivery layer no longer naming concrete providers. The registry still names four concrete providers in its typed fields (`registry/mod.rs:43-46`) and their accessors; erasing those to zero is 7d's outcome, once 7d migrates their callers.
- **Evidence** — the accessor's only callers are the two tests, so removing the trait cannot change production dispatch, because there is none.

**The rule that governs any further deletion**, and the reason this step stops at three: an absent capability is load-bearing. When a kind has no entry in a map, the registry returns that capability's `AdapterError`, and a caller may depend on that. Before any *further* map is removed, a test must assert the absent-capability path for that capability. The three deletions here are exempt only because none has a capability-dispatch path to preserve — `AdapterIdentity`'s map has a reader, but it is name resolution, not capability dispatch, and it is rewritten rather than dropped — that is what makes them safe, and it is the only thing that does.

`AdapterPostBindingFollowup` has exactly one implementor. One implementor is not grounds for deletion: a capability genuinely held by one adapter is still a capability, and the repository's "no single-implementation trait" rule is about speculative abstraction, not about a closed set where one member happens to opt in. It stays.

#### Step 5 — Decompose `AppState` into per-handler ports

*Requires Steps 2 and 3. Blocked without them: decomposing a struct whose fields are reachable by 46 bypasses regresses the moment someone adds the 47th.*

- **Observed** — 40 public fields at `state.rs:58`. The name occurs 511 times across 26 of the 46 modules (23 of the 29 directories): 214 as `&AppState`, 138 as `State<AppState>`, 99 as an import, 30 in declarations and signatures.
- **Proposed responsibility** — a handler's signature states the ports that handler uses, and nothing else. No type in the tree means "everything the process has" except the composition root, which no handler names.
- **Proposed change** — **no context structs.** Each handler takes the ports it uses, extracted from the root by `axum::extract::FromRef`: `State(tasks): State<Arc<dyn TaskDao>>` in place of `State(state): State<AppState>`. The root remains one struct, held by the router, named by the composition root and by nothing else.

  This is chosen over per-band context structs because a context struct is a smaller `AppState` with the same failure mode. It requires a grouping judgement for all 40 fields, that judgement can itself contain a cycle, adding a field to a context silently widens every handler that already takes it, and nothing in the type system says a handler uses only some of what its context holds. `FromRef` needs no grouping decision at all, and a handler's dependencies become exactly what its parameter list says.

- **Evidence — the extractors already work this way twice.** `extractors.rs:157` (`OptionalScope`) and `sse/routes.rs:51` (`SseAuth`) are `impl<S: Send + Sync> FromRequestParts<S>`; `OptionalScope`'s doc says outright it is "state-agnostic … for routers whose state is **not** `AppState`". The three that are not — `AuthUser` (`:59`), `MaybeAuthUser` (`:130`), `ScopeContext` (`:173`) — bind to `AppState` for exactly one value between them: `AuthUser` reads `state.config.sandbox.task_token_secret` at `:97`, and the other two delegate to `AuthUser`. Everything else those impls need comes from `parts.extensions` and request headers. So all three become generic over `S` with a `FromRef` bound carrying a one-method token-verification port, and the `sandbox::jwt` call at `:105` goes behind that port — which also closes `extractors → sandbox`.

- **Consequence for Step 10.** With handlers naming ports, `extractors` stops being a thing every handler file imports as a precondition of compiling, and handler files stay in the crate their domain lives in. The alternative — treating the handler band as a crate — means relocating 47 files and 17,803 lines. Measured on the target graph, the only cross-crate edges that remain from handler files are four `use crate::extractors::AuthUser;` imports, in `access_control.rs:10` (a Step 8 relocation already), `feedback/handlers.rs:7`, `file_storage/handlers.rs:12` and `secret/handlers.rs:7`. Those four resolve by moving `AuthUser` and `AuthIdentity` into the identity crate with the generic impls, leaving verification behind the port. **The Rust orphan rule is why the type and its `FromRequestParts` impl must travel together**, and it is why this is a type move rather than a re-export.

- **Evidence — the pattern is already stated in the code, and stopped one move short.** `task/reaper.rs:617` declares `AgentStatusPort` with the rule written out: *"the one interior read it needs, declared consumer-side so the provider handle stays off `AppState`"*. `sandbox/lifecycle/reconciler.rs:110`'s `NotifyPort` is the same shape, implemented at `observe.rs:207`. Both are correct as ports and both then went **onto** `AppState` anyway, at `state.rs:70` and `:73` — the concrete provider handle left and an `Arc<dyn Port>` took its place, so the god object survived with better field types. Each has exactly one consumer: `state.agent_status` is read at `reaper.rs:458` and nowhere else, `state.sandbox_notify` at `notify.rs:12` and nowhere else. Two fields of forty, one caller each. They are the proof the direction works, not evidence it is done, and this step finishes them by handing each to its single consumer at construction.

  **The two are not equivalent, and only `agent_status` finishes in the structural PR.** `agent_status`'s reader is the reaper, reachable from the composition root through `spawn_task_reaper`, so the port is threaded through the reaper and handed over at construction with no handler touched. `sandbox_notify`'s single reader (`notify_sandbox_for_task`) is reached only through `ensure_sandbox_and_notify(&AppState)`, which has seven handler/adapter/loop callers (`linear/{comments,issues,sessions}.rs`, `event_handler/agent_handlers.rs`, `bindings/runtime.rs`, `loops/firing/schedule_runner.rs`, `message/service.rs`) and also reads `state.tasks`; `GrpcNotify` is connector-backed and no connector is on `AppState`, so removing the field forces each of those callers to take the port — which is the per-handler sweep this step's structural PR does not start. **The `NotifyPort` field removal therefore belongs to the sweep slice that migrates `ensure_sandbox_and_notify`'s callers, not to the structural PR.** This is recorded so "finishes them — plural" does not read, after the structural PR, as one port done and one field abandoned with no owner.

- **`AppState.db`** is carried into a handler signature wherever that handler calls a raw-pool query function predating the DAO layer — `State(db): State<DbPool>`, handed out by a `FromRef<AppState> for DbPool` impl alongside the DAO ports. This is a named residue, not a new grant. The call sites are the same population Step 3 deliberately left alone (`list_adapter_apps`, `upsert_adapter_app`, `delete_adapter_app`, `bot_users_by_app_key`, the `workspace/git_sync.rs` pool helpers); converting them to DAOs is that step reopened, not Step 5 scope. The `FromRef` impl names the pool for those call sites — it is not sanction to run new SQL from a handler. DAO construction still happens only in the composition root, and raw-pool services keep their own pools as Step 2 sets out. Each sweep slice reports how many `State<DbPool>` parameters it adds and the running total, so the final count is chosen rather than discovered.

- **`message/send.rs:111`** reads `state.recently_sent` and `:127` reads `inbound_presentation_by_name`. Under this step that becomes a named port rather than a field: given an adapter name and an external id, what other ids could this event be stored under. That is self-echo deduplication, and it should be modelled as such rather than as access to the whole adapter registry.

- **`bindings` reads twelve ports because it is five jobs sharing a `mod.rs`, not one job needing twelve.** When Step 5's sweep reaches `bindings`, the union of its `state.*` reads is twelve — {adapters, task_bindings, messages, loops, adapter_connections, tasks, task_turns, recently_sent, config, s3, files, event_receipts} — the largest single count in the sweep. The twelve do not belong to one handler: `runtime` (inbound routing + fan-out), `outbound` (thread/reply sends), `reply` (external replies), `status_service` (binding status) and `handlers` (the HTTP surface) each read a coherent slice, and the union is large only because L1's ten modules share one directory — the same shape `adapters/models.rs` had before 7a. Per-handler ports make each slice visible; whether `bindings` is then one crate or several is a Step 7/10 question, and the L1 table is where that split would be drawn. Recorded so the twelve reads read as five jobs made visible, not one god-handler.

- **Band 5 (loops) outcome, and two files deferred to their owning slices.** The loops sweep cleared `loops/firing/inbound.rs` (per-function ports `adapters`, `task_creation`) and gave `loops/firing/listen.rs` a `Connector` over its three ports {`adapters`, `adapter_connections`, `loops`}. The struct does not narrow arity — clippy counts the `&self` receiver, so trading a single `state` parameter for `&self` is net-zero, and the two surviving `too_many_arguments` frames stay suppressed; the struct is earned by the overlap discriminator, not by removing width. Per-function ports would lose on one of those two, not on the file: `connect_binding_optional_subscription` (struct 8 = `&self` + 7 data) would regrow to 10, carrying as explicit parameters the three things the `Connector` holds for it — `adapter_connections` and `loops` for its two resolve helpers, `app_state` for `write_binding`. But `write_binding` (struct 9 = `&self` + 8 data) is a wash: it forwards the whole `&AppState` into `bindings::runtime::upsert_binding`, so per-function ports trade `&self` for one `app_state` parameter and it lands at 9 either way. That `upsert_binding(&AppState)` call is also why `listen.rs` stays on the baseline — the same downstream-facade shape as the `NotifyPort` note above. The dead `Connector::connect` subtree (`connect` → `connect_binding` → `resolve_connection_and_subscription`, no callers workspace-wide) was deleted in-band, retiring the third suppression. Two loops files do **not** clear in this band:
  - **`loops/http/handlers.rs` is deferred behind `adapters/common.rs`.** Four of its handlers (`create_loop`, `update_loop`, `get_loop`, `list_loop_recent_tasks`) forward `&state` into `ensure_repository_exists`, `load_adapter_connection_for_auth` and `load_loop_for_auth`, all `pub(crate)` in `common.rs`, itself on the baseline. `common.rs` is not a facade needing design: its five `&AppState` functions each read exactly one port — `ensure_repository_exists`→`repositories`, `load_adapter_connection_for_auth`→`adapter_connections`, `load_loop_for_auth`→`loops`, `ensure_task_access`→`tasks`, `resolve_connection`→`adapter_connections` — a mechanical five-signature rename. What keeps it out of band 5 is its callers: **26 sites across 6 files** (`ensure_task_access` alone is 13 in 5), most in the adapters band and after; pulling them into band 5 inverts the band order. Count by the `use crate::adapters::common::{…}` import, not by a grep for the function names: `resolve_connection` exists three times under three owners (`common.rs`, `listen.rs`'s `Connector`, `adapters/delivery/typed.rs`), so a name census double-counts — six files carry the import (`adapters/handlers.rs`, `github_pr/api_routes.rs`, `linear/api_routes.rs`, `bindings/handlers.rs`, `bindings/reply.rs`, `loops/http/handlers.rs`); the namesakes carry none. Owned by the adapters band, where the rename clears `common.rs` and `http/handlers.rs` together rather than by design work.
  - **`loops/firing/schedule_runner.rs` is deferred behind `sandbox/notify.rs`**, per the `NotifyPort` note above. It forwards the whole `&AppState` into `ensure_sandbox_and_notify` — one of that function's seven callers — and reads `message_send`, `loops`, and `tasks` directly while forwarding `adapters`+`task_creation` to `inbound`. None of that is a bundle over a shared read (its own reads are disjoint from `inbound`'s, which it only forwards), so per-function ports are the shape whenever the notify-caller slice removes the leak; converting it in band 5 would clear no baseline and retire no suppression.

#### Step 6 — `BackendSandboxServiceImpl`, `Config`, and the `Option`-as-mode pattern

*Requires Step 2. Independent of Step 5.*

- **Observed** — 26 fields, 20 `Option`, at `grpc_server.rs:105-176`; assembled at `startup.rs:517`; `backend_config: Option<Arc<Config>>` at `:152` for three values; `Config` has 20 fields; `registry/bootstrap.rs:20` returns `Option<Self>` whose `None` arm is unreachable.
- **Proposed responsibility** — a service holds the dependencies it uses, at the types it uses them.
- **Proposed change** — the 20 `Option`s resolve one of two ways, decided per field rather than in bulk: a dependency that is genuinely always present loses the `Option`; a dependency that is absent in a real deployment mode moves behind a type that names that mode. `bootstrap.rs:20` returns `Self`, not `Option<Self>`.
- **Evidence** — `grpc_server.rs` names `AppState` once, in a doc comment at `:1439`, so this decomposition is independent of Step 5 and does not wait on it.

- **Step 6 outcome.** Databaseless is a real, codebase-documented mode: `ports: Option<&Ports>` (unset `DATABASE_URL`) drives fourteen of the twenty `Option`s to `None` together. They split by the rule the constructions enforce — *a projection is earned when every construction populates it wholly or not at all* (construction-shape, stronger than read-overlap because it is about an invariant the composition root guarantees, not convenience at a call site). Two projections hold: `HandlerDbPorts` (the eleven the request handlers read) and `IngressPorts` (the three durable-admission writers), because the ingress test harness `for_ingress_tests` wires exactly the three without the eleven, and `service_with` wired two of the eleven without the three (inert — deleted first, as its own commit, so no construction populates a fragment). `credentials_svc` is built from `provider_tokens`, so it is not a `Ports` field and can only live *inside* `HandlerDbPorts`; that is what kills reusing `Option<Ports>` outright rather than on a ratio. The construction table after: production → both `Some`; `for_ingress_tests` → handlers `None`, ingress `Some`; `service_with` → both `None`.
  The remaining six `Option`s are independently gated (events/REDIS, s3+s3_bucket/S3, github_app_service/login-App, task_token_secret/dev-bypass) and stay `Option` — genuinely absent features, not the databaseless mode. `backend_config` is the exception: always `Some` in production (`startup.rs` wraps it unconditionally), so the `Option` drops to a plain `Arc<Config>`. The earlier "three values → three fields" was wrong twice — it counted `prompt::build_for_task`'s three reads, and this struct's *own* use is one read (`sandbox.user_token_ttl_secs`) plus two whole-`Config` forwards to the two prompt builders. This service is the outermost caller of both, so there is no frame above to narrow the forward to; narrowing the builders to the fields they read belongs to their own module, not here. Field count: twenty `Option`s → two projection `Option`s + five genuine-feature `Option`s + one de-`Option`ed `Arc<Config>`.

`auth/routes.rs:145`'s `pub tasks: Option<Arc<dyn crate::task::dao::TaskDao>>` is the same pattern and is settled here. `AuthState::new` at `:162-163` is the only constructor and always fills it with `Some(Arc::new(PgTaskDao(pool.clone())))`, and the only consumer is the preview-URL access check at `:813-826`. The `Option` is dead: it becomes a plain field, or the check takes the port as an argument. Either way the `identity → tasks` edge that Step 10 counts disappears here.

**Config sections do not move between crates.** All ten production references from the runtime name the composite `Config` as a parameter type; none names `DockerConfig` or `ProxyConfig`. Relocating sections changes zero call sites and buys nothing. What is worth doing is narrowing the *parameters* — a function taking `&Config` to read three fields takes those three values instead — and that is a call-site change, not a module move.

#### Step 7 — Untangle `bindings` and `adapters`

*Requires Step 5 for the `footer.rs` half of 7a — it takes `&AppState` at `footer.rs:60`. Wants Step 3 first, because two of the four moves land in files whose DAO construction Step 3 rewrites. 7b, 7c and 7d depend on none of Steps 1–6. Ends with one new crate and the `adapters`↔`bindings` pair broken.*

**Shipped by this stack: 7a, 7b, the `direct_send.rs` deletion, and the untangle baseline. Not 7c.** 7a extracted `moatless-adapter-model` (`models`, `dao`, `parse_adapter_kind`, and `footer`'s pure render/parse layer) behind a transitional facade in `adapters/mod.rs`; 7b moved `subscription_context`/`binding_context` down, cutting `capabilities.rs → bindings` for real; the dead `bindings/direct_send.rs` was deleted, cutting one bindings→adapters file outright. `scripts/check-bindings-adapters-baseline.sh` now ratchets the whole edge in CI — five files still name `crate::adapters` (`handlers`, `outbound`, `reply`, `runtime`, `status_service`) and the count can only shrink; this is the compiler-less enforcement the facade cannot give the data-half kill. **7c did not ship: it folds into the deferred error-split step (see the deferral note below), because its ports would return `adapters` capability types that cannot move until `AdapterError` does, and a port introduced now inverts nothing.**

- **Observed** — `adapters`↔`bindings` is the second-largest mutual pair in the graph at 41/32, and it is not one fault but three:

  1. **`adapters/models.rs` is misnamed.** It is not adapters' models; it is the shared vocabulary of both modules — 19 public items, `TaskBinding` used 94 times in `adapters` and 84 in `bindings`. Seven of the twelve `bindings` files (`dao`, `handlers`, `reply`, `recipients`, `schemas`, `status_service`, `runtime`) import *only* `models`, `error`, `common` or `dao` from `adapters`. Living inside one of its two consumers is the whole data half of the cycle. **(Correction, measured at build against the import lines rather than the module graph: the premise is false for two of the seven — `status_service` also names `AdapterBindingStatus` (`:17`) and `runtime` also names `test_support` (`:866`), both outside those four. And the "stop naming adapters" outcome it feeds is the combined result of 7a, 7c *and* the deferred `common.rs` sweep, not 7a alone. By the end of 7a exactly three files — `dao`, `recipients`, `schemas` — name no `crate::adapters`. The other six were `outbound` and `status_service` (the concrete-registry inversion, since folded into the error step); `handlers` and `reply` (the `common` sweep); `runtime` (the error-test move); and `direct_send` — since **deleted as dead code**, cutting that file outright. So five remain, tracked by the ratchet in `scripts/bindings-adapters-baseline.txt`.)**
  2. **Two pure functions sit on the wrong side.** `bindings::runtime::subscription_context` (`runtime.rs:29`) and `binding_context` (`:55`) are non-async, take only references, and convert `SubscriptionMatch → LoopContext` and `TaskBinding → BindingContext` — vocabulary to vocabulary. They are why `capabilities.rs:230-231`, the file that *declares the provider contract*, reaches up into the binding runtime.
  3. **Providers fetch themselves out of the registry that owns them.** All four typed accessors — `registry/mod.rs:207` `github_pr()`, `:212` `linear()`, `:217` `slack()`, `:222` `telegram()` — return **concrete** types backed by the four `Option<Arc<ConcreteAdapter>>` fields at `:43-46`, and exist only to serve provider-internal calls: `providers/github_pr/handlers.rs:31`, `merge_conflict.rs:92`, `post_merge.rs:150`; `providers/linear/handlers.rs:39`, `api_routes.rs:531`; `providers/slack/handlers.rs:32`; `providers/telegram/handlers.rs:34`, `poller.rs:86` (the last over an `Arc<Adapters>` the poller holds). So `registry → provider` and `provider → registry` both hold for github_pr, linear, slack and telegram alike.

- **Proposed responsibility** — four layers, each answering one question. The vocabulary says what a binding, a connection and an inbound message **are**. The binding runtime says where a message **goes**. An adapter says how to **reach** one external system. The server says what is **mounted**.

  ```
  L3  server         route assembly, composition root
        │
  L2  adapters       capability contracts · registry · ingress · delivery
        │                                                  │
        │                                          providers/* (5)
        ▼                                                  │
  L1  bindings       binding runtime · routing · fan-out · recipients
        ▼                                                  ▼
  L0  adapter-model  vocabulary · connection store · pure conversions
  ```

  | Layer | Holds | Lines |
  |---|---|---:|
  | **L0** `adapter-model` | `models.rs` 506 · `dao.rs` 267 · `footer.rs`'s pure render/parse layer (the `task_comment_footer` DAO wrapper stays up in `bindings`, ≈150 of 207) · the port-facing half of `error.rs` · `parse_adapter_kind` from `common.rs` · the two conversions ≈50 | ≈1,050–1,200 |
  | **L1** `bindings` | `runtime` `recipients` `reply` `outbound` `recently_sent` `status_service` `dao` `handlers` `schemas` `metrics` | ≈3,900 |
  | **L2** `adapters` | `capabilities.rs` 380 · `registry/` 941 · the rest of `error.rs` and `common.rs` · `ingress` `inbound_delivery` `subject_events` `delivery/` `signature` `metrics` `test_support` | ≈5,400 |
  | **L2** providers | github_pr 9,991 · linear 9,528 · slack 6,240 · telegram 4,252 · webhook 786 | 30,797 |

  **`error.rs` and `common.rs` do not move whole, and the reason matters.** Neither is vocabulary. `adapters/error.rs` is an error-conversion hub: it wraps `RepositoryError` (`:82`), `AuthError` (`:90`), `TaskCreationError` (`:108`) and `MessageError` (`:112`) as `#[source]`, imports `AccessControlError` (`:8`), and renders through `crate::error::error_response` (`:9`). It sits *above* four other domains, not below two. `adapters/common.rs` imports `access_control` (`:11`), `loops::dao` (`:15`) and `AppState` (`:16`) and returns `task::models::Task` (`:223`) — handler support, not vocabulary; only `parse_adapter_kind`, which is what `bindings` actually imports, is L0-shaped.

  So both split rather than move, and `error.rs` splits **the same way Step 1 splits `SandboxError`**: the variants the port returns go down, the conversions from other domains' errors and the axum `IntoResponse` stay up. `models.rs` and `dao.rs` go down, but **not untouched — the "intact" premise was written from the module graph, not the import lines.** `models.rs` carries four `crate::` references, not the two doc-links the graph showed: three doc-comment paths (`:16`, `:310`, `:381`) de-pathed to bare backticked names, one intra-doc link (`:426`) repointed at `task_comment_footer`'s new L1 home, and one **code** reference (`:319 crate::adapters::AdapterKind`) repointed at `moatless_api_schemas::enums::AdapterKind`. `dao.rs` names `crate::db::DbPool` (→ `sqlx::PgPool`, a bare alias) and `crate::adapters::models::AdapterConnection` (→ `crate::models::…`), and carries no doc-links. The workspace denies no `rustdoc` lint, so the broken doc-links would have rotted silently; this move is the pass that surfaced them. **Dependency delta (recorded, not absorbed):** the L0 crate is not free-standing — it takes `moatless-backend-core` and `moatless-api-schemas`, both with the `database` feature, because `Scope`, `RoutingMode` and `AdapterKind` live in those crates and their `sqlx` impls (and `AdapterConnection`'s `FromRow`) are `database`-gated. Two leaf deps below L0, stated as the rule rather than left unwritten.

  **The `error.rs` split is deferred to its own step after this stack (ruled).** In order:
  - `adapter-model` needs no `AdapterError` at all — verified, the crate names it nowhere: `dao.rs` returns `sqlx::Error` at every signature, and `models`, `footer` and `parse_adapter_kind` return no error. So nothing about the crate extraction is waiting on this; that is what makes the deferral safe rather than merely convenient.
  - A two-type split alone does not even cut the `bindings → adapters::error` edge it was meant to. `bindings/runtime.rs:305` (`AdapterError::TaskCreation(e)`) and `:794` (`.map_err(AdapterError::MessageDelivery)`) construct two backend-domain-sourced variants in production, whose sources (`TaskCreationError`, `MessageError`) L0 cannot name — they belong in the *up* type. So `runtime.rs` would still name `crate::adapters::error::AdapterError`, and would name two error types where it names one today. Four of five bindings files stop naming `adapters::error`; the one that matters most does not.
  - The prior question is a design one a mechanical split cannot answer: does `bindings` own an error type of its own, with `adapters` converting into it at the boundary? `runtime.rs:305`/`:794` say the binding runtime's real failure modes include "creating a task failed" and "delivering a message failed" — not port errors that leaked. This must be settled *before* any variant moves, or the split gets done twice.
  - Cost: 491 `AdapterError::` construction sites across 62 files in `backend/src`, with `IntoResponse` and `source`-chain snapshot tests asserting exact rendering and exact preserved sources.
  - The `runtime.rs` `error_source_tests` move (offered as a 7a bonus) belongs to the error step's scope — it cannot move before the type it tests does.
  - **7c folds into this step (ruled).** The prior question is the same one: does `bindings` own an error type? Until it is answered the capability traits cannot move below `AdapterError`, and until they move a `bindings`-declared `OutboundProviders`/`BindingStatusSource` returns `adapters` types and inverts nothing — a single-implementation port bought for a rename, against the pragmatic rule. So the concrete-registry inversion of `outbound.rs` and `status_service.rs` waits on the error type and rides this step. It inverts **two** live files, not three: `direct_send.rs` is already deleted as dead code, which cut the third for real.

  These are line counts at `HEAD`, not at the `8fe60cd` measurement base the module graph uses; `adapters/` reads 37,414 here against 37,242 there, and `bindings/` 3,916 against 3,913. The band arithmetic is re-derived against one base, not against these.

- **Proposed change** — four moves and one port.

| | Move | Kills |
|---|---|---|
| 7a | Lift `models.rs` and `dao.rs`, `parse_adapter_kind` out of `common.rs`, and `footer.rs`'s pure render/parse layer once Step 5 has taken `&AppState` off it, into a new `adapter-model` crate. The `error.rs` split moved to its own later step — see the deferral note above. `AdapterKind` needs nothing — `adapters/mod.rs:31` already re-exports it from `moatless_api_schemas::enums`. | The data half. Of the twelve `bindings` files, three (`dao`, `recipients`, `schemas`) stop naming `adapters` by the end of 7a; the other six wait on 7c, the `common` sweep, or the error-test move (see the correction under Observation 1). |
| 7b | Move `subscription_context` and `binding_context` down beside the types they convert. | `capabilities.rs → bindings`. The provider contract stops depending on the binding runtime. |
| 7c | `bindings` declares `OutboundProviders` and `BindingStatusSource`; `Adapters` implements them. | **Deferred into the error-split step (did not ship).** The ports return `adapters` capability types that carry `AdapterError` and cannot move until it does, so a port now renames `&Adapters` and inverts nothing — see the deferral note. |
| 7d | Delete all four typed accessors — `registry/mod.rs:207` `github_pr()`, `:212` `linear()`, `:217` `slack()`, `:222` `telegram()` — and the four `Option<Arc<ConcreteAdapter>>` fields at `:43-46` that back them; what remains of the per-provider handle is the `dyn Adapter` already in the map. Each provider's handlers receive their own adapter through router state; `telegram/poller.rs` takes `Arc<TelegramAdapter>`. Eight callers migrate: `github_pr/handlers.rs:31`, `merge_conflict.rs:92`, `post_merge.rs:150`; `linear/handlers.rs:39`, `api_routes.rs:531`; `slack/handlers.rs:32`; `telegram/handlers.rs:34`, `poller.rs:86`. | `registry ↔ provider`, in both directions, for all four providers. |

  **7a lifts only `parse_adapter_kind` from `common.rs`.** The pure DTO builders (`adapter_connection_response`, `task_binding_response`, `binding_descriptor`) are L0-shaped but stay: moving them touches their `adapters`/`bindings` callers without taking any file to zero, so they ride the deferred `common.rs` sweep. `loop_response` cannot move at all — it reads `crate::loops::dao`. `parse_adapter_kind` is re-exported from `common` so the `loops`/`task` callers keep their path; only `bindings/schemas.rs` names the crate directly, which is what takes it to zero.

  **7c is narrow on purpose — but narrow is not delivering (corrected).** The port names only what `bindings/outbound.rs` calls — `outbound_for_new_thread_by_name` (`:53`), `outbound_for_binding_reply_by_name` (`:270`), `destination_builder` (`:98`) — not `dyn Adapter`, and that shape is right. What the shape cannot fix: those three methods return `&dyn AdapterNewThread`/`AdapterBindingReply`/`AdapterDestinationBuilder`, `status_service.rs`'s port returns `&dyn AdapterBindingStatus`, and all four capability traits return `Result<_, AdapterError>` (`capabilities.rs:121,163,196,207,214,238,249,256`), so a `bindings`-declared port returns `adapters` types and cannot stop `outbound.rs`/`status_service.rs` from naming `crate::adapters::capabilities::*` and `::error`. It renames `&Adapters` to a single-implementation port with no test mock (against the repo pragmatic rule) and inverts nothing. And `direct_send.rs` was not "32 lines that come along free" — it was dead code (never constructed anywhere in `backend/**` or `crates/**`; `pub` hid it from the `dead_code` lint) and is deleted, which cut that file for real. The inversion of the two live files rides the error step.

  **The footer splits (Option D); its pure layer needs no port, its wrapper does.** The original claim — "`footer.rs` is 207 lines of `render_footer` … it goes to L0 whole" — was false: `footer.rs` is two things in one file. `render_footer`, `parse_task_id_from_body` and `FOOTER_MARKER` are a string render and its inverse — no adapter behaviour, no port — and go to L0 with the vocabulary; the ~six provider call sites keep `crate::adapters::footer::…` through the same facade the model types use. But `task_comment_footer` is not pure: Step 5's footer prerequisite already gave it a `&dyn TaskDao` port to load the task name, so it is binding-runtime behaviour, not vocabulary, and stays up in `bindings::footer`. The providers and `outbound.rs` that call it now reach **down** into `bindings` (L2→L1), which removes `outbound.rs`'s old *upward* reach into `adapters::footer`. "Does this provider carry a footer" remains a capability question `capabilities.rs` already models.

  **`adapters/routes.rs` is not inverted; it disappears.** `:9` and `:28-72` mount `bindings::handlers::*` inside the adapters router. Route assembly is L3's job and L3 may name both, so this reach is removed by relocation rather than by a port.

- **Evidence — the directions that already point the right way and do not change.** Inbound: `providers/linear/handlers.rs` → `providers/linear/ingress.rs` → `adapters/inbound_delivery.rs:106` → **down** into `bindings::runtime::route_inbound_message`. Fan-out: `adapters/subject_events.rs:34,:39` → **down** into `bindings::runtime::fan_out`. Binding upsert: `adapters/handlers.rs:17-18` → **down** into `bindings::outbound` and `bindings::runtime`. Only the outbound reply path points upward, and 7c is the whole of it.

**One crate now, the other two later.** `adapter-model` is extracted in this step: its job is to be depended on by both sides, so Cargo rejecting an edge back up is the entire point, and it is ≈1,650 lines with no async and one DAO. `bindings` and `adapters` stay modules. After 7a–7d their direction is already correct and already checked by the compiler — a trait does not compile against its implementations — so splitting them buys enforcement that mostly exists and costs the `bindings::handlers` route wiring. That call is taken later, on evidence, in the re-derived Part III.

**7d is not contingent on any of that.** A provider reaching back through the registry that holds it is wrong on its own terms, and it is the single fact that decides whether five provider crates are possible at all. It ships whether or not the rest of this step does.

---

---

#### The target: fifteen crates

Step 7 builds `adapter-model`; Steps 8 to 11 build the rest. It is stated once here so all four steps can refer to it.

`backend/src` becomes fifteen crates, each taking whole modules — no module is split across two crates except the root files, which divide three ways.

| Crate | Lines | Takes from `backend/src` |
|---|---:|---|
| `moatless-core` | 8,062 | `config.rs` `error.rs` `db.rs` `events.rs` `metrics.rs` `feature_flags.rs` `shared_models.rs` `scope.rs` `lib.rs` `test_support.rs` `leader.rs` · `file_storage/` `task_event/` `task_listing/` `tag/` `secret/` |
| `moatless-identity` | 12,220 | `auth/` `extractors.rs` `access_control.rs` `team/` `adapter_apps/` |
| `moatless-workspaces` | 11,470 | `workspace/` `repository/` `skill/` `plugin/` `credentials/` |
| `moatless-tasks` | 17,998 | `task/` `message/` `task_activities/` `task_turn/` `agent/` `event_ingress/` `feedback/` |
| `moatless-sandbox-control` | 18,664 | `sandbox/lifecycle/` and `sandbox/*.rs` |
| `moatless-sandbox-k8s` | 12,820 | `sandbox/k8s/` |
| `moatless-sandbox-docker` | 3,359 | `sandbox/docker/` |
| `moatless-adapter-model` | ≈1,200 | the L0 slice of `adapters/` that Step 7a lifts |
| `moatless-loops` | ≈13,800 | `loops/` `bindings/` · `adapters/` minus `adapters/providers/*/` minus the L0 slice |
| `moatless-adapter-github-pr` | 9,983 | `adapters/providers/github_pr/` |
| `moatless-adapter-linear` | 9,479 | `adapters/providers/linear/` |
| `moatless-adapter-slack` | 6,200 | `adapters/providers/slack/` |
| `moatless-adapter-telegram` | 4,234 | `adapters/providers/telegram/` |
| `moatless-adapter-webhook` | 784 | `adapters/providers/webhook/` |
| `moatless-server` | 9,766 | `main.rs` `startup.rs` `state.rs` `supervisor.rs` · `sse/` `t3code/` `event_handler/` `setup/` `provider_settings/` |

Total 140,055 — today's `backend/src`, unchanged in size, only in shape. The `adapter-model`/`loops` division of the old 15,016 is approximate until Step 7a's three splits are cut.

This module→crate mapping is the **input** to `tools/crate-census` (`bands.rs::crate_of_backend_mod`), not a restatement of its output, so it stays here. It is not cheaply reducible to one literal source — one is a markdown table a human reads, the other a Rust `match` — so the rule is a same-commit invariant instead: **any change to this table and `bands.rs::crate_of_backend_mod` land in the same commit.** Ruling A moved `leader.rs` to core in both.

`access_control.rs` is under identity rather than core because its only content is a helper over `AuthUser` (`access_control.rs:10`); leaving it in core is the single edge that would make core depend on identity.

> **The crate-level accounting below is withdrawn.** An independent critic review reproduced
> the module-level analysis exactly - 140,055 lines, 305 ordered pairs, the 29-module SCC, all
> five mutual-pair counts, the 40-bypass table - and falsified the crate-level numbers. Two
> defects in the counting: references into the seventeen root files were discarded (561 of them,
> including `crate::error` 249 and `crate::state` 99), and the pattern matched only `crate::`,
> so 773 `use super::`/`use self::` statements were never seen. Those are exactly the references
> that cross sub-module boundaries, which is what a crate boundary is made of.
>
> Consequences: **1,133 / 1,034 / 99 / 27 are wrong** (independent recounts give 121-213 illegal
> references in 28-34 classes); **the acyclicity claim is false** - at least six illegal edge
> classes appear in no step, and each closes a cycle with a verified downward edge, so bands
> 10a-10e and 10g all stall; **Step 5's `AuthUser` move is direction-wrong** for the three core-band
> handlers that import it (`file_storage`, `secret`, `task_event`), making `core <-> identity` a
> crate cycle that 8g widens; **Step 8h's two halves import each other**; **Step 11b's grep-zero
> gate contradicts the Untouched list**, since `moatless-vibe-backend` is also the JWT issuer,
> the GitHub User-Agent, the Kubernetes field manager and the Deployment, Service and OTel
> service names.
>
> Steps 1-6 are unaffected: every anchor and count they rest on was independently reproduced.
> Do not carve a crate against the numbers below until they are re-derived with sub-module and
> `super::` resolution, and every illegal reference is pinned to a named step the way Steps 8
> and 9 already do.

**Re-derived — `tools/crate-census`.** The accounting above is re-derived by a syn resolver that follows `crate::`/`super::`/`self::`/`use` and `pub use` re-export chains to definition sites, scans attribute and macro token streams and doc comments as separate classes, and scores each reference against the 15 target-crate bands (`adapter-model` placed below `identity`, confirming 9f). It is the Step 10 ratchet — `scripts/crate-census-baseline.txt`, checked by `scripts/check-crate-census.sh` in CI — and is deleted at 10i, when Cargo enforces the bands. Its one blind spot, macro-generated paths, is audited to zero and re-inventoried on every run; the only `macro_rules!` in scope are local SQL builders that emit no path. Full output: `scripts/crate-census-report.txt`.

Today's counts (production, cross-crate): **238 illegal Cargo edges** — 206 CODE, 16 attribute-embedded, 16 macro-embedded (Ruling A, built below, took the four `LeaderGuard` edges off); the per-`(source → target)`-class breakdown is the report's, not restated here. Intra-doc paths are a separate class, never a Cargo edge and never in this total: 52 cross-crate doc paths, of which 8 point upward across the band order (real carve work — an up-pointing `///` link stops resolving the moment the boundary exists, as Step 8f showed — but listed in their own section of the baseline, out of the ratcheted count, so an illegal total cannot move when a comment is edited). Removing exactly the 238 turns the target-crate graph from **one 14-crate strongly-connected component into a DAG** (Tarjan, both runs in the report) — the acyclicity claim holds only once they are gone. The count runs above the earlier 121-213 estimate because the AppState sweep (Step 5) is in flight: 66 of the 238 are live `crate::state::AppState` edges the estimate assumed already ported. (A file-based `#[cfg(test)] mod ...;` — `auth/test_support.rs`, `adapters/registry/tests.rs` — is scored test-context, not production, so it does not inflate the total.)

**A precondition is met when the census says the edge is zero, not when the step it names is marked shipped.** Three shipped steps left residue a later step assumed gone: Step 5 shipped and 66 edges are still live AppState; Step 1 shipped and made the `FleetObserver` port singular and provider-free, but — not being a placement step — left its two provider adapters (`K8sFleetObserver`, `DockerFleetObserver`) inside the control-band `sandbox/lifecycle/observe.rs` directory, so `sandbox-control` reads as fanning out (Ruling F relocates them, a placement, not a missing abstraction); Step 3 shipped and the codex refresher bypassed the `LeaderGuard` port (now closed by Ruling A). Read every "requires Step N" precondition against the census, not the ship log.

**All 238 references are now pinned to a named step; zero are unpinned.** The last 7 — the `sandbox-control -> sandbox-docker`/`-k8s` refs in `backend/src/sandbox/lifecycle/observe.rs` (lines 15-18, 54, 84) — are not a control-band site fanning out: they are the two `FleetObserver` provider adapters (`K8sFleetObserver`, `DockerFleetObserver`, `observe.rs` header and `:39`/`:103`) sitting in the control-band directory. Ruling F relocates each into its provider crate; the `FleetObserver` port and its reconciler stay in the control band. Pinned to 10f, the placement step for exactly this defect in both directions.

Rulings that repriced the other 19 formerly-unpinned refs:
- **Ruling A (LeaderGuard, Step-3 residue) — BUILT.** The `LeaderGuard` port exists (`leader.rs`) for exactly this. The codex refresher bypassed it (`workspaces -> sandbox-k8s` ×3); `schedule_runner` used it correctly but `leader.rs` was a root file the table mapped to `server`, so it read `loops -> server` ×1. The trait moved to `core` (dependency-free; `leader` now maps to core), `K8sLeaderGuard` moved beside the k8s client (`sandbox::k8s::leader_election`), and the refresher now takes `Arc<dyn LeaderGuard>` built at the composition root. Four edges gone, no new abstraction — the trait gained a second consumer of the shape it already had.
- **Ruling B (10f, repriced).** Eleven of the eighteen `sandbox-control -> provider` refs are the 10f defect in the control→provider direction: non-Kubernetes helpers under `k8s/` — workspace-config parsing (`auto_provision`), failure-reason strings (`pod_status`), `StorageBackend` — moved down to `sandbox-control`. 10f's stated content was three symbols; the census shows fourteen. Pinned to 10f.
- **Ruling C (8g, handler placement).** `file_storage`, `secret`, `task_event` each own a `handlers.rs`, and `auth/routes.rs`, that reach `identity` (`AuthUser`) and `tasks` (`TaskDao`). An HTTP handler taking an `AuthUser` and a `TaskDao` is server-band by nature: one handler-placement question, not four edges.
- **Ruling D (9e, reopened).** `tasks -> adapter-github-pr` (`message/hooks::reconcile_shell_command`) is the consumer-declared-hook shape (the `AgentErrorSink` precedent — one impl, declared by the consumer, registered at the root), not the `Adapter`-trait method that `shell_reconcile.rs:34-36` refuses. Still open, still blocking 10d.
- **Ruling E (jwt placement).** `identity -> sandbox-control` — `auth/routes.rs` verifies a *user* auth token via `sandbox::jwt::verify_user_auth_token`; that is identity work sitting under `sandbox/` only because sandbox tokens are minted there too. Placement move.
- **Ruling F (FleetObserver adapters, placement).** The 7 `sandbox-control -> sandbox-docker`/`-k8s` refs in `observe.rs` are the two `FleetObserver` provider adapters (`K8sFleetObserver:39`, `DockerFleetObserver:103`) living in the control-band directory, not new fan-out. The port already exists in `lifecycle/reconciler.rs`. Move `K8sFleetObserver` into `moatless-sandbox-k8s` and `DockerFleetObserver` into `moatless-sandbox-docker`; the `FleetObserver` trait and its reconciler stay in `sandbox-control`. Both adapters depend down on `sandbox-control` (the port, `ObserveError`, `RuntimeObservation`) and on `tasks` (`SandboxDesiredState`, `TaskStatus`); they are independent of each other, so no sibling edge appears. All 7 edges die, no abstraction invented — pinned to 10f. Unpinned drops to 0.

**10a's precondition is not "none".** `moatless-core` has 15 illegal outgoing references — 10 to `identity` (the `AuthUser`/`access_control` handlers in `file_storage`, `secret`, `task_event`), 4 AppState routers to `server`, 1 `TaskDao` to `tasks` — so it does not compile as a crate until Step 5, the 8g handler placement, and the `TaskDao` holding are resolved. Carve readiness is a **frontier**, re-run after each carve: a crate is carvable only when it has no illegal outgoing edge *and* every crate it depends on is already extracted. Only `core` (the base case, depending on the floor alone) is decidable today, and it is blocked; `sandbox-k8s` and `server` have no illegal outgoing edges but still depend on un-carved crates, so they read WAITING, not READY. See the report's carve-readiness table.

**The band order.** Cargo edges point from a crate to the crates it depends on, so a legal edge always runs from a higher band to a lower one:

```
band 0  core
band 1  identity      adapter-model       ← siblings
band 2  workspaces
band 3  tasks
band 4  sandbox-control
band 5  sandbox-k8s   sandbox-docker      ← siblings
band 6  loops
band 7  adapter-github-pr  -linear  -slack  -telegram  -webhook   ← siblings
band 8  server
```

**Crates in the same band are siblings and may not depend on each other**, which is the whole point of a provider band: two providers that reference each other are one provider wearing two names. `sandbox-docker → sandbox-k8s` is the only sibling edge that exists today, and it is real — see Step 10f. `adapter-model` is placed beside `identity` because it depends on `core` and on nothing else once Step 7a's splits are done; that placement is a claim the re-derivation has to confirm, not one this revision has measured.

Against that order the current code has **1,133 cross-crate references, of which 1,034 are legal and 99 are not** — 27 edge classes, 8.7% of the total. Removing exactly those 99 makes the crate graph acyclic by Tarjan; nothing else has to move.

| Where each of the 99 dies | Refs |
|---|---:|
| Steps 1, 4, 5 and 6, as a by-product of work already justified on its own terms | 51 |
| Step 8 — placement | 35 |
| Step 9 — a consumer-owned port | 11 |
| Step 10f — the misfiled sandbox helpers | 2 |

**A module cycle is not a crate cycle.** Steps 1–6 leave a 26-module strongly connected component, and that is not a contradiction: Cargo rejects cycles between packages, not inside one. Grouping does the work. Of the 45 mutually-referencing module pairs, several stop being constraints because both ends land in one crate.

**`adapters`↔`bindings` is the exception, and this spec no longer absorbs it.** An earlier revision argued the 41/32 pair needed no inversion because both ends landed in `moatless-loops`. That reasoning was sound about Cargo and wrong about the code: the pair is three separable faults, not one dependency, and the third — providers fetching themselves out of the registry at `registry/mod.rs:240,:245` — is what decides whether five provider crates are possible at all. Step 7 breaks the pair for that reason rather than for a packaging one, and it is the only place this program breaks a module cycle it could have grouped away.

**Band convexity is what makes the grouping legal, and it is a real constraint on any regrouping.** A crate may span a contiguous run of bands; it may not skip one. Merging two modules with anything between them that routes both ways creates a cycle no port can remove. That is why `agent/` and `event_ingress/` are in `moatless-tasks` rather than in a crate with `workspace/` and `repository/`: those two sit below `task/` and these two above it, and a crate holding all four re-creates the cycle the split exists to remove. Any later regrouping has to re-run the check, not reason about it.

**Naming: the directory is bare, the package carries `moatless-`.** That is what eight of the eleven existing crates already do — `crates/events` is `moatless-events`, `crates/backend-core` is `moatless-backend-core`, `crates/t3code` is `moatless-t3code`. `claude-agent-sdk` and `codex-sdk` are exceptions because they mirror upstream names. `sandbox-proto` is the only unexplained one and is not changed by this program. `moatless-vibe-backend` is the other name that breaks the rule, and Step 11 is when it stops existing.

#### Step 8 — The eight placement moves

*Wants Part II done so the tree is quiet, but depends on none of it. Still one crate at the end of this step.*

Each is a piece of code sitting in the wrong module. Moving it removes a cross-band edge and changes no behaviour. Per `AGENTS.md`, each is its own commit, and each lands before anything else touches the file.

| # | Edge removed | Refs | The move |
|---|---|---:|---|
| 8a | `identity → adapter-github-pr` | 10 | Every reference is one call: `auth/github/service.rs` → `adapters::providers::github_pr::api::api_base()`. The GitHub API base URL goes to `moatless-core`. |
| 8b | `identity → workspaces` | 7 | `GitHubScope` and `CodexScope` (`credentials/github/scope.rs`, `credentials/codex/scope.rs`) are OAuth scope enums used by `auth/provider_token_dao.rs:7-8`, `auth/credentials.rs:25`, `auth/test_support.rs:11-12`. They move to `moatless-identity`; the credential *service* stays in workspaces. |
| 8c | `tasks → loops` | 12 | `task/connections_handlers.rs` is entirely binding handlers, and `task/handlers.rs:22-23,248,1556` builds binding summaries. Both are loops code filed under `task/`; they move to `moatless-loops`. |
| 8d | `workspaces → server` | 2 | `credentials/codex/auth_json.rs:10-11` imports `ProviderSettingsError` and two Codex schemas. Those three types move down to `moatless-core`. |
| 8e | `sandbox-control → server` | 1 | `sandbox/preflight.rs:120` calls `provider_settings::github_client::check_github_repo_access`. The GitHub client moves to `moatless-core`. |
| 8f | `core → tasks` | 1 | `task_event/dao.rs:35` re-exports `task_turn::models::UnclosedTurn`. `UnclosedTurn` moves to `task_event`, and `task_turn` re-exports it instead. The two doc links at `:32,:79` follow. |
| 8g | `core → identity` | 1 | `access_control.rs:10` uses `AuthUser`; the file moves to identity. |
| 8h | `identity → server` | 1 of 2 | `auth/routes.rs:21` imports `evaluate_setup` and `SetupContext`, used by `me_handler` at `:875-884` to fold setup status into `/auth/me`. `setup/evaluator.rs` (93 lines) and `setup/schemas.rs` reach nothing above identity — their `SetupContext` takes `provider_tokens` and `secrets` — so they move to `moatless-identity`. `setup/github.rs` (216 lines) is the half that reaches `sandbox` and `credentials`, and it stays in `moatless-server`. |

The second reference in 8h's class is `extractors.rs:21`'s `use crate::state::AppState`, which dies in Step 5 rather than here.

Order does not matter among these; they touch disjoint files. 8f is the one to read carefully — inverting a `pub use` changes which crate owns a type, and a stale doc link compiles. 8h is the one to size before starting: it is a module split, not a file move, and the 93/216 line division is the claim to check first.

**Step 8 outcome — four moves shipped, four deferred; the eight-move count was three real moves plus one narrowing.** The census (whole tree — `backend/src`, `backend/tests`, `crates` — classified by source band) found four of the eight priced moves kill the edge they name, and repriced the rest. Shipped as four commits:

- **8a shipped.** `api_base` (and `DEFAULT_API_BASE`) was the sole `identity → adapter-github-pr` carrier (`auth/github/service.rs`); it moved to `config.rs` (core). Edge to zero.
- **8e shipped, and dragged its error.** `github_client.rs` was the sole `sandbox-control → server` carrier (`sandbox/preflight.rs`); it moved to `github_client.rs` (core). `GitProviderError` is shared with `gitness_client` (which stays in server), so it moved to a neutral core module `git_provider.rs` rather than into `github_client` — putting it in `github_client` would make `gitness_client` reach up into it. `gitness_client` and `ProviderSettingsError` import it from `git_provider`. Edge to zero. Depends on 8a.
- **8f shipped.** `UnclosedTurn` moved from `task_turn::models` to `task_event::models` (core); `task_turn::models` re-exports it, so `reaper`, `event_ingress` and `task_turn::dao` are unchanged. The two upward doc links (`dao.rs` `active_turn_for_task`, and the type's own doc) were demoted to plain code spans. `core → tasks` to zero.
- **8d phantom half shipped, error half deferred.** The two Codex schemas were already in the `api-schemas` crate (`provider_settings::schemas` is only `pub use moatless_api_schemas::provider_settings::*`); `auth_json.rs` now imports them there directly. This **narrows** `workspaces → server` from three symbols to one — it does not remove it. `ProviderSettingsError` stays (see the cluster below).

**Not shipped — repriced or defective:**

- **8b does not kill its edge.** The two scope enums are not the only `identity → workspaces` carriers: `auth/routes.rs:2004,2006` call `credentials::codex::parse_codex_auth_json`/`build_codex_metadata`. Moving the enums leaves those two calls. Deferred to the cluster below.
- **8c as written has no referent — ruled.** The `tasks → loops` edge has six carriers, and none is "binding code filed under `task/`": `task/closer.rs` and `task/reaper.rs` hold `Arc<dyn TaskBindingDao>` as a task-close/reap port; `task/presenter.rs` holds it plus `Arc<LoopDao>` to assemble a `TaskResponse`; `message/send.rs` holds `Arc<RecentlySent>`; `task/handlers.rs` reads `get_bindings_for_tasks`/`TaskBindingSummary` into the task list; `task/connections_handlers.rs` is a cross-cutting admin view that also reaches `adapters` (band 7, above loops). Moving any to loops drags task/message domain code up into loops (`loops → tasks`; for `connections_handlers` also `loops → adapters`) — the band-convexity cycle Step 10 forbids. So the placement move the plan describes does not exist. Its real content is two things: **(1)** one genuine misfile, `connections_handlers.rs`, whose home is the **server** band, not loops — a separate placement decision; **(2)** a **`tasks`/`message → loops` consumer-owned port** (Step 9 shape) for the other five. `TaskBindingDao` is **already a trait with a test mock** (`handlers.rs` `FakeBindings`), so this clears the single-implementation bar that stopped 8h — the trait exists; the inversion moves its *declaration* down from `crate::bindings::dao` to the consumer band.
- **8g is a no-op as written — plan defect.** `access_control.rs` is already identity-band, so moving it removes no `core → identity` edge. The real carriers are three core-band handlers importing identity's `AuthUser`: `file_storage/handlers.rs:15`, `secret/handlers.rs:9`, `task_event/handlers.rs:11`. The fix is an `AuthUser` placement decision (push the extractor down, or lift those three handlers), not a file move. This is the direction-wrong `AuthUser` placement the withdrawal block flags. **Do not attempt 8g under Step 8** — it belongs to the Step 5 `AuthUser` question.
- **8h is defective — the two halves import each other.** `evaluator.rs:8` imports `super::github::GithubTokenRequirement` and hard-constructs it in `checks()`; `github.rs:6` imports back `SetupContext`/`SetupRequirementCheck`; and `github.rs` reaches `sandbox` (`GITHUB_TOKEN`) and `credentials` (`GitHubScope`). So "`evaluator` reaches nothing above identity" is false, and no cut along the file boundary kills `identity → server` without re-creating an equal-or-worse edge. The only cut that works inverts the check registry so `evaluate_setup` receives its checks from a server-band composition root — **but that entrenches `SetupRequirementCheck` as a single-implementation trait, which `AGENTS.md` bars.** **8h lifts only when `SetupRequirementCheck` earns a second implementation or is collapsed.** Do not attempt under Step 8. (Line 621's second member of 8h's class, `extractors.rs:21`'s `use crate::state::AppState`, is a Step 5 death, unrelated to this split.)

**The deferred error/credential cluster — one knot, one entry condition.** Four deferrals point at the same place and are tracked together, not as four independent laters: (1) the `error.rs` split, already ruled to its own step; (2) 7c's `OutboundProviders`/`BindingStatusSource` inversion, which folds into the error split because its ports carry `AdapterError`; (3) 8b's remainder — the `credentials::codex` parse/build path `auth/routes` calls; (4) 8d's error half — `ProviderSettingsError`, which wraps `AuthError` (identity) and `CodexOAuthError` (workspaces) while being `IntoResponse`, so it cannot descend to core. Entry condition: the error split lands first and gives each of these a home its callers are already below. Until then none inverts anything, and each would buy a single-implementation seam against the pragmatic rule.

#### Step 9 — The six consumer-owned ports

*Requires Step 8 for 9a, which lands in a file 8c moves. Still one crate at the end of this step.*

**Re-derive the `Refs` counts before trusting them — they price how many times module A names module B, not which symbol carries the edge.** Step 7 was priced as seven sub-steps and delivered under a different shape; Step 8 was priced as eight moves and is three real ones (8a, 8e, 8f) plus one narrowing (8d). Both were derived from the same module-dependency graph, which records *that* A names B and never *which* symbol, so a move that relocates the named symbol can leave the edge standing behind a second carrier — that is exactly what happened to 8b, 8c and 8g. Steps 9–11 come from that same graph. Whoever starts Step 9 re-runs the census per edge — whole tree (`backend/src`, `backend/tests`, `crates`), classified by source band, every call site and hand-built value followed — before costing or building, rather than trusting the `Refs` column. This is now a measured pattern, not a run of bad luck.

These are calls upward that survive the placement moves. The caller declares the trait it needs; the higher crate implements it. **The call edge and the dependency edge point opposite ways** — that is the whole mechanism, and it is why these are declared by the consumer rather than exported by the provider.

| # | Edge inverted | Refs | The port |
|---|---|---:|---|
| 9a | `tasks → server` | 3 | `task/reaper.rs:29-30` and `event_ingress/mod.rs:41` reach into `event_handler`. `moatless-tasks` declares what it needs of agent-event handling — `agent_handlers`, `AgentEvent`, `is_agent_message`; `moatless-server` implements it. |
| 9b | `workspaces → sandbox-control` | 3 | `credentials/mod.rs:34-35` needs `SandboxConnector` and `GITHUB_TOKEN`; `workspace/git_sync.rs:39` needs `ServerConfig`. These are the same secret-injection port in two places, and it is declared once. |
| 9c | `workspaces → tasks` | 1 | `credentials/mod.rs:39` holds a `TaskDao` to find which tasks a credential refresh affects. It takes a narrower port naming that one query. |
| 9d | `workspaces → loops` | 1 | `repository/handlers.rs:14` calls `sync_repo_loops` directly. Repository sync publishes; `moatless-loops` subscribes. |
| 9e | `tasks → adapter-github-pr` | 1 | `message/hooks.rs:22` calls `reconcile_shell_command`. That function does not move — it is GitHub semantics — so the call site inverts: `moatless-tasks` declares the hook, `moatless-adapter-github-pr` registers against it at the composition root. |
| 9f | `identity → loops` | 2 | `auth/routes.rs:149` holds `Arc<dyn AdapterConnectionDao>` for one purpose: the Linear OAuth grant upserts a workspace-level `adapter_connections` row at `:2337-2368`. Step 3 removes the `PgAdapterConnectionDao` construction at `:191`, but the trait itself lives in loops, so the edge survives it. `moatless-identity` declares the one-method port it actually needs — record an adapter connection for this grant — and `moatless-loops` implements it. `:2367`'s test import follows. |

**9f is the one that looks like it is already handled and is not.** `auth/routes.rs:191` is one of the 46 bypass sites Step 3 closes, so a reader tracking that list will tick this edge off early. Closing the bypass replaces a concrete type with a trait; it does not change which crate declares the trait.

Doing these inside the single crate is deliberate. A port that is wrong is cheapest to fix while both sides still compile together; discovering it during the crate carve means unwinding a `Cargo.toml`.

**Step 9 outcome — one real port shipped; five of the six priced ports evaporate on census.** The per-edge re-run (whole tree, definition-site followed, not the `Refs` column) found only 9a survives as a port. It shipped as four commits, three of them enabling placement moves the port needs:

- **9a shipped as the `AgentErrorSink` port.** The 9a row priced three `tasks → server` symbols; only **one was an edge**. The real shape is **one inversion plus two placement moves down**: `AgentEvent` + `EventKey` and `is_agent_message` were plain agent-event vocabulary sitting a band too high, so they moved *down* to a new tasks-band `agent_event` module rather than inverting (a backend-only Redis-parse model — it does **not** belong in the shared `moatless_events` crate, which the sandbox also compiles). **`event_ingress`'s use is satisfied by the move, not a port.** The one genuine upward call — the reaper's orphaned-turn error path forwarding the whole `AppState` to `agent_handlers::handle_agent_error` — inverts through a consumer-owned `AgentErrorSink` trait (declared in `reaper.rs`, `AppState` implements it in the server band). `handle_agent_error` **stays server-band**: it reaches `state.tasks/events/linear_active_turns`, then `notify_parent_task_of_child_error` → `task_relations` → `sandbox::notify::ensure_sandbox_and_notify`, running into the sandbox band — it cannot descend. The port takes `reaper.rs` off `scripts/appstate-composition-root-baseline.txt` (67 → 66). This is the phantom lesson from the other direction: the table counted symbols and only one of them was an edge — Step 10's registration lists must be priced the same way. `ServerConfig` (the 9b half below) moved down in the same chain.
  - **Coverage gap (placed against 10d — see the Step 10 note).** The 9a unit test (`close_orphaned_turn_as_error_delivers_reconcile_event_to_sink`) covers only the helper: the reconcile event's shape and that the sink fires exactly once. It does **not** cover the reaper's invocation of that helper from `close_orphaned_turn` — deleting that call leaves the unit green while orphaned turns silently stop closing and the UI stays blocked. It is not Step 9's work to build a harness for it, but it is not unplaced either: it closes as a side effect of **10d**, because of what the reaper is made of (below).
- **9b — the `SandboxConnector` half is a phantom; the `ServerConfig` half is a placement move (shipped in the 9a chain).** `SandboxConnector` is defined at `crates/backend-core/src/sandbox/mod.rs:208` — the L0 floor, already a two-implementation trait; `workspaces → backend-core` is downward-legal, so there is no edge to invert. The row read a `crate::…SandboxConnector` reference as a `workspaces → sandbox-control` edge, but the definition is in the floor. The `ServerConfig` half is real but is a placement, not a port: this is the preview-server entry in `sandbox::models` (**not** `config.rs`'s port config — two same-named types), and it moved down to `repository::models` beside `ResourceConfig`, so `agent/prompt` (tasks), `workspace/git_sync` (workspaces) and the sandbox providers (sandbox-control) all reach down or sit at the same band.
- **9c — not a port; a deferred responsibility refactor (see 10c below).** `broadcast_to_scope` (`credentials/mod.rs:407`) holds a `TaskDao` for one query of twenty-four, auto-invoked from `store()` behind an `Option<BroadcastContext>` that credentials' own tests always leave `None`. That is behaviour in the wrong module, not a boundary needing a port. Recorded against 10c with its stronger shape and entry condition.
- **9d — stays open (see 10g below).** `sync_repo_config` (`repository/handlers.rs:1126`) is not the misfiled cross-cutting handler `connections_handlers` was: its body runs `sync_repo_loops` (loops) **and** `log_workspace_sync` (workspaces-private) and returns `(Repository, LoopSyncSummary, WorkspaceSyncSummary)` — half workspaces-native, so it does not move.
- **9e — dead.** `message/hooks.rs`'s `reconcile_shell_command` port is refused in writing at `shell_reconcile.rs:34-36`; declaring it re-introduces the abstraction the code rejected.
- **9f — phantom.** `AdapterConnectionDao` is defined at `crates/adapter-model/src/dao.rs:10` — the L0 floor, sole impl `PgAdapterConnectionDao`; `identity → adapter-model` is downward-legal. The row read `crate::adapters::…` as a `identity → loops` edge, but `adapters/mod.rs:39` re-exports the `moatless_adapter_model` crate — the reference is a facade, not the definition.

**The facade-laundering phantom pattern — three instances of one mechanism: a `Refs` count that read a facade re-export as if it were the definition.** 9f (`AdapterConnectionDao` via `crate::adapters::`), 9b-`SandboxConnector` (floor trait named through workspaces), and 8d before them (the Codex schemas re-exported through `provider_settings::schemas`). Any `Refs` count reading a known re-export surface — `crate::adapters::dao`/`models` (re-exports `moatless_adapter_model`), `provider_settings::schemas` (re-exports `moatless_api_schemas`), a `task_turn::models` re-export of a `task_event` type — is suspect until the definition site is checked. **Steps 10 and 11 are costed from the same `Refs` column and inherit the same defect**; the carve's two-independent-assembly-sites lists (routers, ApiDocs, workspace members, the composition root, CI job matrices) are the same shape a definition-site census, not a symbol-found census, is required to price.

**9c and 9d are not the same problem, though both are a workspaces-band call reaching upward.** 9d needs the sub-sync result *synchronously* (the admin who triggered the sync must see a `LoopSyncSummary` or a failure), so its only fix is a server-band composition. 9c's fan-out is *fire-and-forget delivery*, so its fix is a contract change (below). Do not group them.

#### Step 10 — Carve the crates

*Requires Steps 5, 7 and 8. This is the step the program exists for.*

The `Cargo.toml` files appear one band at a time, lowest first, because a crate can only be extracted once everything it depends on is already a crate.

| # | Crate | Precondition beyond the band below |
|---|---|---|
| — | `moatless-adapter-model` | already a crate — Step 7a extracts it, and it is listed here only because the bands below depend on it |
| 10a | `moatless-core` | **8g handler placement** (Ruling C — analysis pending). Edges: report |
| 10b | `moatless-identity` | **8b credential/error cluster**; **8g handler placement**. Edges: report |
| 10c | `moatless-workspaces` | the **9c broadcast refactor** (below); **9d** (→ 10g). Edges: report |
| 10d | `moatless-tasks` | **8f**, **9a**, **9e** (reopened hook), the **`tasks`/`message → loops` port** (8c's real content — not a placement move), and the **`TaskReaper` field inversion** (below). Edges: report |
| 10e | `moatless-sandbox-control` | **Ruling F** relocates the two `FleetObserver` provider adapters out of the control-band directory (a placement, done at 10f); the port itself is provider-free (Step 1). Edges: report |
| 10f | `moatless-sandbox-k8s`, `moatless-sandbox-docker` | **Placement moves**: the non-Kubernetes helpers under `k8s/` down to `sandbox-control` (Ruling B), and the two `FleetObserver` provider adapters from `observe.rs` into their provider crates (Ruling F). Edges: report |
| 10g | `moatless-loops` | **8c**, **9d** (server-band composition, below), **Step 7** (`bindings` must not name `Adapters`). Edges: report |
| 10h | the five `moatless-adapter-*` | **Step 7d** (registry field-widening + accessor deletion). Edges: report |
| 10i | `moatless-server` | everything above |

The precondition column names **only the design work the census cannot compute** — the refactors, placements, and reopened decisions below. It carries no edge counts, symbol lists, or "requires Step N" that `tools/crate-census` already scores; "Edges: report" means `scripts/crate-census-report.txt`'s carve-readiness table is the current, machine-generated authority for what edge each carve must first drive to zero. Two hand-maintained copies of what the census computes would disagree eventually and the reader could not tell which is current — the same rule that keeps `docs/deferred-changes.md` from rotting, applied here.

**10c precondition — the 9c broadcast refactor (named change, not a port).** `store()` (`credentials/mod.rs:338`) stops broadcasting at all; the two server-band callers (`state.rs:293`, `startup.rs:466` assemble the context; the fan-out is auto-invoked from `store()`, `refresh.rs:68`, `refresh.rs:171`) broadcast explicitly. That deletes the `Option<BroadcastContext>` (`mod.rs:105`, whose doc admits "when omitted, store does not broadcast" — the optional side effect *is* the smell), deletes the duplicated `BroadcastContext` assembly at both composition roots, removes `TaskDao` from credentials, and kills `workspaces → tasks` — one change paying four ways, with no trait anywhere. **Entry condition:** it lands with the fan-out test credentials never had (`broadcast_ctx` is always `None` in their suite, so the loud path is unexercised). Do not smuggle it into Step 9; do not narrow it to hoisting only the task lookup, which leaves the optional side effect in place and buys only the edge.

**10g precondition — 9d is a server-band composition, and that is the only shape.** `sync_repo_config`'s two sub-syncs move to a server-band composition that orchestrates them; `workspaces → loops` dies with no trait. The event-driven alternative is **refuted by the admin-visibility contract**: the handler returns `LoopSyncSummary` to the admin who triggered the sync and must surface a failed refresh synchronously (`repository/handlers.rs:1139-1141`) so a skipped refresh is never mistaken for a successful approval-revocation — a publish can return neither a summary nor a failure to that caller. One option, not two.

**10d precondition — two reaper fields must invert to ports, and doing so closes the 9a coverage gap for free.** `TaskReaper` (`reaper.rs`, tasks band) takes eleven collaborators: **eight are trait-backed ports** — `agent_status`, `messages`, `tasks`, `task_turns`, `task_events`, `task_bindings`, `events` (an `Option<Arc<dyn EventPublisher>>`), and `agent_error_sink` — and **three are concrete**: `config: Arc<Config>`, `k8s: K8sClient`, and `sandbox: Arc<Lifecycle>`. Only two of the three concretes are above the tasks band and block the carve: `k8s` (sandbox-k8s) and `sandbox` (sandbox-control). `config` is a concrete `Arc<Config>` too, but `Config` is core band — a downward dependency — so it is **not** a blocker and needs no port (it is a plain config struct). A `moatless-tasks` crate that names either above-band concrete does not compile, so 10d cannot ship until `k8s` and `sandbox` are ports. This is unpriced work and a hard precondition of the carve, not a cleanup. **The side effect:** once both are ports, `TaskReaper::new` is constructible from fakes for its eight-plus-two ports and a plain `Config`, so the integration test that covers the reaper's invocation of the error sink from `close_orphaned_turn` (the 9a gap) writes itself — the only thing that made it expensive is gone. **Entry condition, therefore: if 10d lands without a test that fails when the `close_orphaned_turn` → sink call is deleted, that is the miss to catch, because at that point it is cheap.** The 9a unit test covers only the event payload; it does not cover the invocation.

At every point the workspace builds and the full gate runs. The crate count goes up and nothing else changes.

After 10i, `backend/` no longer exists as a source directory. `backend/Cargo.toml` becomes `crates/server/Cargo.toml`; the workspace `members` list gains thirteen entries and loses one.

#### Step 11 — The two package renames

*Requires Step 10. Last, because each rewrites command lines rather than code.*

| # | Rename | Sites |
|---|---|---|
| 11a | `moatless-sandbox` → `moatless-sandbox-agent` | 26 — `-p` flags in `AGENTS.md` and five workflows, path dependencies, `Cargo.lock` |
| 11b | `moatless-vibe-backend` → `moatless-server` | `AGENTS.md`'s clippy and test block, `ci.yml`, `scripts/shadow/up.sh:59`, `Cargo.lock` |

`moatless-sandbox` is the in-container agent, so without 11a the three sandbox orchestration packages read as children of it and are not. Of 210 occurrences of that string in the repository, **182 are Docker image, pod and service names and only 26 are the Cargo package** — separate name spaces, so the rename touches no deployment artifact. It leaves the family reading correctly: `-agent` runs inside the sandbox, `-control` orchestrates from outside, `-k8s` and `-docker` are the two providers it dispatches to. Not doing it leaves `-p moatless-sandbox` and `-p moatless-sandbox-k8s` meaning unrelated things in the same command block.

`AGENTS.md` is the file whose clippy block must stay runnable verbatim, so it is edited **in the same commit** as each rename, never after. A commit that renames a package and leaves `AGENTS.md` naming the old one hands the next agent a command that fails.

### Data model

No table, column, index or migration changes anywhere in this program. The state whose identity and consistency does change is in-memory.

```mermaid
classDiagram
    class CompositionRoot {
        -DbPool pool
        +30 port objects
        +registers providers
    }
    class RouterState {
        +FromRef per port
    }
    class AdapterRegistry {
        +HashMap~AdapterKind, Arc~ adapters
        +11 capability maps
    }
    class SandboxRuntimeContract {
        +5 ports
        +SandboxError
    }
    CompositionRoot --> RouterState : builds
    CompositionRoot --> AdapterRegistry : builds
    CompositionRoot ..> SandboxRuntimeContract : selects provider
    AdapterRegistry ..> SandboxRuntimeContract
```

#### AdapterRegistry

`backend/src/adapters/registry/mod.rs:35` — the in-memory capability registry, keyed by `AdapterKind`. Identity is the kind; the invariant is that a kind's presence in a map means it can perform that capability.

| Field | Change | Why |
|---|---|---|
| `user_identities: HashMap<AdapterKind, Arc<dyn AdapterUserIdentity>>` | removed | No production dispatch; only callers are `registry/tests.rs:89,:107` |
| the `AdapterIdentity` map | removed | The blanket impl at `capabilities.rs:101` makes its key set identical to the `Adapter` map by construction |
| the five `Option<Arc<Concrete>>` fields at `:46-49` | `Arc<dyn>` | A registry that names `GithubPrAdapter` cannot live in a crate below it |
| all others | unchanged | An absent key is load-bearing — see the Step 4 rule |

#### Composition root

New. Holds the `DbPool`, the constructed ports, the selected sandbox provider, and the registered adapter providers. Its lifecycle is the process; its invariant is that **it is the only place a `Pg*Dao` is constructed** and the only place a concrete provider type is named. It is not the only holder of a `DbPool` — raw-pool services legitimately hold one and keep doing so, per Step 2. That construction invariant is what the Step 3 CI check enforces, and it is the only reason the port abstractions constrain anything at all.

#### Router state

One struct, held by the router, named by the composition root and by no handler. It implements `FromRef` for each port a handler extracts, so `State<Arc<dyn TaskDao>>` resolves without the handler naming the root. Its field count is not a design target — nothing reads it whole, so a field added to it widens nobody.

#### `task_listing` — a rename and a signature change this program does not do

Not part of any step. Recorded because Step 10 is where it stops being cosmetic, and rediscovering it there is expensive.

`task_listing` is a change feed with two subjects (`models::ChangeSubject`), not a task-domain module. Of the nine calls into it from outside `task/`, seven are `record_workspace_change`: `workspace/dao.rs:266,:435,:459,:494,:692`, `workspace/git_sync.rs:610`, `repository/handlers.rs:489`; the other two are `tag/dao.rs:51,:106`. The name predates the second subject. That is also why it sits in `moatless-core` rather than `moatless-tasks`.

The direction of those edges is fixed by the design and cannot be inverted. `dao.rs:1-23` requires the record to be written on the mutation's own `&mut PgConnection` — a pool would commit immediately and make the record visible before the mutation it describes — and requires the position to be allocated in that same transaction from one deployment-wide counter row held until commit, because an id assigned at INSERT and made visible at COMMIT can be read out of order and a polling reader will skip a record silently while reporting itself up to date. So it is neither an event emitted afterwards nor a port a higher module calls: every mutator of listing-visible state calls it directly, and it calls nothing. That makes it a leaf that several domains write.

Two changes it needs:

- **A name with one subject in it.** `task_listing` reads as task-owned and it is not; a reader who takes the name at face value will place it wrong.
- **An ordering rule the signature enforces.** `dao.rs:13-23` states it in prose: take every other lock this transaction needs before calling this, and none afterwards. The counter is one row the whole deployment serialises on, so a transaction that takes it before a `tasks` row is on the opposite side of a lock order from every writer that takes it after, and the pair deadlocks under concurrent load. A doc comment is adequate while every caller compiles in one crate. **It is not adequate across a crate boundary**, and the fix is to stop exporting a bare `record_change(&mut tx, …)` in favour of a wrapper that owns commit ordering, so "last" is not a thing a caller can get wrong.

### Interfaces and contracts

**External contracts — none change.** The HTTP API, the generated `@moatless/api-client` consumed by `apps/frontend`, the backend↔sandbox gRPC protocol, the T3 UI-RPC surface served by `crates/t3code`, every adapter's inbound webhook handling and outbound delivery, and every database schema are unchanged. This program is invisible from outside the process.

**Operational names are also a contract, and are also unchanged.** Docker image names, Kubernetes pod, Service and PVC names, and every label are untouched by Step 11. The two things renamed are Cargo package identifiers, which appear only in `-p` flags, path dependencies and `Cargo.lock`.

**Internal interfaces.** Everything below is a proposed implementation boundary, not a promise; changing one later needs no spec update.

| Interface | Consumer | Today | After |
|---|---|---|---|
| Five sandbox runtime ports | `crates/t3code` (`files.rs:29`, `review.rs:29`, `terminal.rs:28`), `backend/src/credentials/mod.rs:34`, `sandbox/lifecycle/observe.rs:13`, `task/reaper.rs:625`, `startup.rs:423`, `state.rs:31` | two declarations, bridged by `t3code/backend.rs` | one declaration |
| Sandbox port error | the same set, plus `sandbox/client.rs:61-79` | two types, converted both ways | one type, no `kube`/`bollard` in its shape |
| `TaskBindingDao`, `AdapterConnectionDao` | 22 files, 40 sites | port declared, concrete type used | port only; concrete constructor private |
| Composition root | `main.rs`, `startup.rs`, `auth/routes.rs` | three independent assemblies | one |
| Handler state | 511 references naming `AppState` | one 40-field struct | the ports each handler names |
| Agent-event handling | `task/reaper.rs:29-30`, `event_ingress/mod.rs:41` | direct call into `event_handler` | consumer-owned port, Step 9a |
| Sandbox secret injection | `credentials/mod.rs:34-35`, `workspace/git_sync.rs:39` | direct call into `sandbox` | consumer-owned port, Step 9b |
| Loop sync on repository change | `repository/handlers.rs:14` | direct call to `sync_repo_loops` | publish/subscribe, Step 9d |
| Shell-command reconciliation | `message/hooks.rs:22` | direct call into `github_pr` | consumer-owned hook, Step 9e |
| Adapter-connection upsert on OAuth grant | `auth/routes.rs:149,:2337-2368` | `Arc<dyn AdapterConnectionDao>`, the full DAO | one-method consumer-owned port, Step 9f |
| Setup evaluation | `auth/routes.rs:875-884` | `setup::evaluate_setup`, a call upward | `setup/evaluator.rs` moves down to identity, Step 8h |

The 511 `AppState` references and the 46 bypass sites are listed as blast radius, not as contracts. They are mechanical call-site updates.

### Failure, safety and recovery

The program changes no runtime failure behaviour. The failure modes worth naming are the ones the *work* can introduce.

| Condition | State before | State after | What the caller sees | Mitigation |
|---|---|---|---|---|
| A step lands with a behavioural change slipped in | green | green, behaviour differs | nothing until production | Each step is reviewed as a pure refactor. A diff that changes a `match` arm or an error mapping is out of that step's scope and is split out |
| The unified port error collapses two variants callers distinguish | 22 + 7 variants | fewer | a caller's `match` takes a different arm | Before merging Step 1, every `match` on either error type is enumerated and each arm mapped explicitly. `sandbox/error.rs:337`'s test moves and is extended to cover each provider-classified variant |
| Step 3 promotes a DAO method with different semantics than the concrete one | correct query | subtly different query | wrong data, silently | Promoted methods keep the concrete implementation verbatim and the trait method delegates to it, exactly as `bindings/dao.rs:137-264` already does for its existing fifteen |
| Step 4 deletes a map whose absence path a caller depends on | typed `AdapterError` | compile error or wrong dispatch | 500, or an adapter silently doing nothing | The Step 4 rule: no map is deleted without a test asserting its absent-capability path. The three deletions there are exempt only because none carries capability dispatch |
| Step 6 removes an `Option` that is genuinely `None` in a supported deployment | starts, degraded | fails at startup | process does not start | Each of the 20 is decided individually against whether a real deployment leaves it unset. "Always present" is established from `startup.rs:517`'s assembly, not assumed |
| Step 7a lifts a type whose `#[cfg(test)]` module names a private sibling | green | does not compile | the extraction stalls on tests, not on code | `adapters/models.rs` and `dao.rs` move whole for this reason. The two files that split, `error.rs` and `common.rs`, split at a boundary their tests are checked against first |
| Step 7c's port is shaped around today's three lookups and a fourth appears | green | green | `bindings` grows a method it does not need, or reaches for `Adapters` again | The port names what `outbound.rs` calls and nothing else. A fourth caller is a review question — does `bindings` need this, or does the call belong above it |
| Step 7d hands a provider its own adapter and a second construction site appears | one registry entry | two | a provider configured twice, diverging | 7d deletes `registry/mod.rs:240,:245` rather than adding a parallel path; `registry/bootstrap.rs:49,:58,:71,:89` stays the only construction site |
| A Step 8 move leaves a stale `pub use` behind | green | green | a type reachable by two paths, and the edge not actually removed | Each move's commit deletes the original declaration, not just adds the new one. The edge count is re-measured after Step 9 and must be zero before Step 10 begins |
| Step 10 extracts a crate whose band still has an upward edge | green | does not compile | the carve stalls mid-way | The per-band preconditions in Step 10's table. A band that will not extract means a Step 8 or 9 item was missed, and the fix is that item, not a `Cargo.toml` workaround |
| Step 11 renames a package and leaves `AGENTS.md` stale | green | green | the next agent runs a `-p` flag that does not resolve | `AGENTS.md` and the workflows are edited in the same commit as each rename |
| A rebase against a moved `main` produces a diff against the wrong base | — | — | phantom deletions in review | Every step's review states its merge-base explicitly. This has produced three false findings in this engagement already; it is the single most common review error here |

**Recovery.** Every step is one branch, one PR, revertable independently. Steps 1, 2, 4, 6, 7, 8 and 11 are independently revertable outright. Reverting Step 3 after Step 5 has landed would strand Step 5, so if both are in flight they revert together. Step 10's per-band commits revert in reverse order; reverting 10a alone is not possible once 10b has landed.

**Degraded operation is unchanged.** Whatever the process does today when the database is unreachable, it does after. Step 6 may make databaseless *startup* fail fast where it currently starts with 20 `None` fields — that is a deliberate change, called out in Behaviour above, decided per field rather than assumed.

### Testing and verification

**Per-step gate.** Every step, before Step 11 renames anything:

```
cargo clippy -p moatless-vibe-backend --all-targets -- -D warnings
cargo clippy -p moatless-backend-core --all-targets --all-features -- -D warnings
cargo clippy -p moatless-backend-core --all-targets --no-default-features -- -D warnings
cargo clippy -p moatless-t3code --all-targets -- -D warnings
cargo test -p moatless-vibe-backend
task test:integration          # Taskfile.yml:340 — the real database tier
```

The clippy lines are run verbatim as `AGENTS.md` specifies; each flag set keys a separate rustc incremental unit, so varying them recompiles the dependency graph. `task test:integration` is the database entry point. A narrower form — `cargo test -p moatless-vibe-backend --features db-tests --lib` — is documented at `backend/src/sandbox/lifecycle/contract_tests.rs:11` and is the right loop when iterating on one contract test.

`moatless-backend-core` is linted both ways because its `database` feature gates the pairing module, and Step 1 touches that crate's `sandbox` module: keeping `--no-default-features` green is what keeps the feature droppable.

**During Step 10 the gate grows one line per crate**, in band order, and `AGENTS.md`'s block is extended in the same commit that adds the crate. The `-p moatless-vibe-backend` line becomes `-p moatless-server` at 11b.

**The E2E suite has a measured baseline of 26 failures on `main`, all predating this program — compare against 26, not zero.** The first full run of the suite in repo history (post-Steps 5–7, run `30980297505`) was 69 passed, 26 failed, 127 skipped; an identical-harness run on pre-stack `main` (`d9e89ce22`, run `30982512558`) failed the same 26 tests, name-for-name in both directions — so Steps 5–7 introduce no behaviour the suite can observe. The 26 are three classes, all tracing to `06615c520` (2026-07-30): 14 "Activating a Loop requires an explicit run-as user" (`loops/http/handlers.rs:166`); 6 "`moat repos create` … required arguments were not provided" (the CLI requires `--git-url`, `crates/cli/src/commands/repos.rs:29-30`); and 6 sandbox/workspace `expect().toBe()` assertions (SANDBOX-01 ×2, SANDBOX-07, SERVER-02, WS-20, WS-30.1). A later step that changes this number owns the change.

**Step-specific verification**

| Step | Proves it worked |
|---|---|
| 1 | Exactly 5 declarations of the sandbox ports exist in the workspace. `t3code/backend.rs` no longer names `backend_sandbox::`. Neither `kube` nor `bollard` appears in the contract's dependency tree. The DTO projections still exist and still narrow — a test pins the projected field set |
| 2 | One function constructs every `Pg*Dao`; `startup.rs` and `auth/routes.rs:191` construct none. `#[sqlx::test]` cases appear in the CI run's test count. **Not** "no `DbPool` outside the root" — raw-pool services keep their pool and are out of scope |
| 3 | The CI check finds zero `Pg[A-Za-z]+Dao(` constructions outside the root and the `dao.rs` files. All 46 sites and the 4 signatures updated. Existing binding and connection tests pass unchanged — they are the behavioural evidence |
| 4 | `registry/tests.rs` passes with the two assertions removed. A test pins `identity_by_name` and `adapter_kind_for_name` resolution for every kind and every provider key, written before the rewrite and unchanged after it. A new test per remaining capability asserts the absent-capability error path. 12 traits, 11 maps, and `registry/mod.rs` names no concrete adapter type |
| 5 | No handler names `AppState`; the only remaining occurrences are the root's declaration and the composition root. Every handler signature names its ports. `message/send.rs:111,:127` reach only the narrowed self-echo interface |
| 6 | `BackendSandboxServiceImpl` has no `Option` field that is not a named deployment mode. `bootstrap.rs:20` returns `Self`. `auth/routes.rs:145` is not an `Option` |
| 7 | `cargo tree` shows `adapter-model` depended on by `bindings` and `adapters`, and depending on neither. `bindings` names neither `Adapters` nor `dyn Adapter`. `capabilities.rs` names no `bindings::` path. `registry/mod.rs` exposes no concrete provider type, and no file under `providers/` names `Adapters` |
| 8 | Re-run the cross-band edge count: the 35 references in the Step 8 table are zero, and no new ones appeared. Every moved item has exactly one declaration — a move that adds a `pub use` without deleting the original scores as done and is not |
| 9 | The 11 references in the Step 9 table are zero. Each new port has a test against a recording implementor |
| 10 | `cargo tree` shows no path from a lower band to a higher one **and no edge between two crates in the same band**. Fifteen packages. The full gate green after each band |
| 11 | `AGENTS.md`'s command block runs verbatim. `git grep -c moatless-vibe-backend` is zero. No Docker or Kubernetes name changed — diff the rendered Helm values and the compose file |

**Where a test lives, and why the split is the lever rather than the move.** 30,857 of `backend/src`'s lines are `#[cfg(test)]` — 53,713 across the workspace — and they are what the second compile of `cargo test -p moatless-vibe-backend` builds. The instinct is to move them to `backend/tests/`, which already exists with about forty files. Measured against what actually forces co-location — does the test module name an item that is private in the same file — that instinct pays badly:

| | files | test lines |
|---|---:|---:|
| Names a private item; **must** stay co-located | 156 | 27,403 |
| Public surface only; **could** move | 108 | 10,957 |

71% is load-bearing. `sandbox/k8s/pod_builder.rs` (1,763 test lines) drives `build_sandbox_container` and `cache_host_parent`; `sandbox/lifecycle/orchestrator.rs` (1,036) drives `allocate_generation` and `commit_fact_and_clear_marker`. Those are private deliberately, and testing them through the public surface would be worse code bought with a faster build.

**So the rule is: co-location is the default, and a test moves to `tests/` only when it already names nothing private.** The 10,957 lines that qualify are opportunistic cleanup, not a step, and no step in this program is gated on any of it.

**The split is what shortens that compile, and it needs no test to move.** The cost is one crate rebuilt with `cfg(test)`; `sandbox/` at 10,652 and `adapters/` at 10,020 are two thirds of the load, and Steps 7 and 10 scatter them across `sandbox-control`, `-k8s`, `-docker`, `adapter-model` and five provider crates. This is stated as a consequence, not a target — see **Out of scope** on performance thresholds.

**Contract tests.** `backend/src/sandbox/lifecycle/contract_tests.rs` is the model for Step 1's shape, not for its subject. It drives the `Lifecycle` orchestrator over the real `PgLifecycleDao` and `PgLifecycleLock` with a *recording* runtime — real database, faked runtime — which is exactly the right pattern: pin the contract against one real implementation and one instrumented one.

Step 1 needs the mirror image, and it does not exist today. The unified ports get a test suite written against the trait, run against a recording implementor, asserting the behaviour each method promises. That the Docker and K8s adapters still satisfy the unified trait is proven by the compiler; that they still *behave* correctly is what the E2E suite is for. The six ports of Step 9 get the same treatment, and that is the only new test infrastructure this program needs.

**What E2E can and cannot gate here.** The two profiles are not interchangeable and neither covers the error mapping.

- **`E2E_PROFILE=k8s` does not exercise a branch by default.** `tests/shared/profile.ts:104-110` resolves `baseUrl` from `MOATLESS_API_URL` or `BACKEND_API_URL`, falling back to the in-cluster Service `http://moatless-vibe-backend.moatless.svc.cluster.local:8080` — whatever image the cluster is running. Pointing it at branch code means standing up the shadow backend, and `scripts/shadow/up.sh:59` runs `cargo build -p moatless-vibe-backend` at the `:33` default profile of `release`. So the k8s profile costs a full backend compile in-sandbox, which is heavier than the `cargo test` link. Under a 4 GiB sandbox memory request it is not runnable at all; `SHADOW_CARGO_PROFILE=dev` is the cheaper form and is still a full build.
- **`E2E_PROFILE=docker` runs off-sandbox.** `.github/workflows/e2e.yml` is `workflow_dispatch` on staging-runners-large: `moat gh workflow run e2e.yml -f playwright_suite=api --ref <branch>`. This is the profile that gates branch code.
- **Neither profile reaches the T3 surface.** `grep -rlniE "ui_rpc|uiRpc|t3code" tests/api/specs/` matches none of the 31 spec files. `t3code::backend::sandbox_error`'s eight call sites are all inside that surface's port impls, so the down-projection has no black-box coverage in either profile.
- **Capabilities differ.** `seedUsers`, `resetDatabase`, `directDatabase`, `inboundCallbacks` and `localImages` are docker-only. The one thing only k8s covers is the `K8sSandboxAdapter` path itself — where `KubeError` and `PodBuildError` originate — and `sandbox-errors.spec.ts` drives provider failure through paths that skip on k8s or do not route through the projection.

So the gate is **per commit, not per step**. Step 1's commit 1 is gated by the four clippy lines and its own unit tests, because nothing observable changes: the up-lift's reachable mappings are unchanged, and `t3code/src/workspace.rs:127` renders `Provider` and `Other` through the same `Sandbox(_)` arm by design, so the user-visible string is identical. The commits that delete port declarations and rehome the projection do change wiring the HTTP surface observes, and those are gated by the docker profile in CI. **A gate that cannot run is worse than a smaller one that can** — it reads as coverage and delivers none.

Steps 8 and 10 are the two most mechanically risky and the cheapest to gate: both are behaviour-preserving by construction, so the existing suite plus `cargo tree` is the whole gate. Step 9 is the opposite — a port can be subtly wrong while everything compiles — and is the one that earns its own tests.

**What only a person can judge**

- Whether the ports a handler names in Step 5 are concepts or just the fields it used to reach through `state`. A signature that lists six ports has made the coupling visible, not smaller; visible is the point, but six is still a question worth asking of that handler.
- Whether a Step 6 `Option` removal is correct, since it encodes a deployment mode nobody wrote down. Reading `startup.rs:517` shows what is assembled, not what a real operator runs.
- Whether the unified port error's variants are the right vocabulary, as opposed to the union of two existing ones.
- Whether each Step 9 port is a concept the lower crate genuinely owns, or a hole punched to make a build work. The test is whether the trait reads as something `moatless-tasks` needs, or as something `moatless-server` happens to provide.
- Whether the groups are the right groups. Band convexity says which groupings are *legal*; it does not say which is best, and a legal grouping can still put two unrelated things together.

## Out of scope

- **The 35 files with raw `sqlx::query` outside a `dao.rs`, and the raw-pool services alongside them** (`adapters/delivery/store.rs:35-52`, `adapters/providers/linear/adapter.rs:38-75`, `sandbox/lifecycle/lock.rs:27-33`, `adapter_apps/mod.rs:125` and the rest). A genuine one-off query, a service that legitimately owns a pool, a migration tool and a test helper are four different things and need classifying before any of them moves. Doing this alongside Step 3's 46 mechanical sites is what would make Step 3 unschedulable.
- **`crates/claude-agent-sdk` and `crates/codex-sdk`.** Their six traits are their own protocol abstractions. Vendored SDKs follow upstream shape.
- **Restructuring the inside of each adapter provider.** Step 10h gives each provider its own crate; it does not change what is in one. Bringing Slack, Telegram, Linear and Webhook to the shape GitHub PR already has is `.plans/remaining-adapter-breakup.md`'s territory, and it runs after Step 4.
- **Renaming `backend/src/adapters/providers/linear/agent_api.rs:55`'s `ActivityKind`.** It is Linear's wire vocabulary and correctly lives with Linear; only the name collides with `backend-core/src/activity/mapping.rs:103`. Worth fixing, not worth blocking a step on.
- **`sandbox-proto`'s missing prefix.** It is the one existing crate whose name breaks the rule for no discoverable reason. Renaming it is a free consistency fix and is not this program's business.
- **CONTEXT.md's "sandbox runtime" double meaning.** `:97-99` defines it as the Kubernetes-or-Docker substrate; `:77` uses it for the in-container half. Step 11's naming works around the collision rather than resolving it, and the glossary fix is a separate change.
- **`reconcile_shell_command` and `github_pr`'s 1,848 lines.** GitHub semantics, not layering. It does not move; only its call site inverts, at Step 9e.
- **Flipping `T3_UI_RPC_ENABLED` to default-on.** A product decision, and the reason it is named here rather than ignored is that it lands on two things this program touches: `router_from_env`'s fail-closed startup behaviour (`task_listing/dao.rs:76`) and the unauthenticated `/oauth/token` path at `auth/middleware.rs:84`. Whether the default flip keeps failing startup on incomplete configuration or degrades to not-mounted with a loud log is a decision that belongs with the flip, not with the refactor.

- **Raising the sandbox memory request.** Real, and orthogonal — an eviction problem, not an architecture one.
- **Any performance target.** The build-time numbers in this document are measurements taken to size the problem. There is no threshold this program must hit, and no step should be shaped to hit one.
