# Proposal: Workspace concept (0..n repositories + servers)

**Status:** Research + proposed design (no code yet)
**Author:** research pass, 2026-07-18
**Related:** [`workspace-dir-multi-repo-persistence.md`](./workspace-dir-multi-repo-persistence.md) (the filesystem-layer prerequisite), [`rename-repository-to-dao.md`](./rename-repository-to-dao.md)

## TL;DR

Today a **Repository** row is two things fused together:

1. **A git pointer** — remote URL, provider, default branch.
2. **A run configuration** — docker image, setup commands, env vars, preview
   servers, plugins, K8s resources, system prompt.

A task references exactly **one** repository (`tasks.repository_id: text NOT
NULL`), and the whole sandbox is provisioned from that single row. You cannot
run a sandbox with **zero** repos (a blank scratch environment) or **many**
repos (frontend + backend + shared lib in one sandbox).

**Proposal:** introduce a first-class **Workspace** entity that owns the *run
configuration* and holds **0..n repository references** and **0..n server
definitions**. A **Repository** becomes a thin, reusable git pointer. A **Task**
references a **Workspace** instead of a Repository. This cleanly enables
zero-repo and multi-repo sandboxes while preserving the current single-repo
experience via a compatibility shim.

The sandbox *filesystem* already supports multiple repos under
`/opt/moatless/workspace/<repo>/` (see the persistence plan); this proposal adds
the *config/data-model/provisioning* layer on top so the system can be **told**
about multiple repos rather than only tolerating ones the agent clones by hand.

---

## 1. Current state (research findings)

### 1.1 The `Repository` model conflates pointer + run-config

`backend/src/repository/models.rs:16-53` — one flat Postgres row
(`repositories` table, `backend/migrations/0001_schema.sql:73-103`) carries all
of:

| Concern | Fields |
|---|---|
| **Git pointer** | `kind`, `remote_url`, `default_branch`, `provider` |
| **Run config** | `docker_image`, `docker_access`, `sandbox_service_account`, `setup_commands` (jsonb), `env_vars` (jsonb), `servers` (jsonb `ServerConfig[]`), `plugins` (jsonb), `resources` (jsonb `ResourceConfig`), `system_prompt` |
| **Legacy preview** | `server_start_commands`, `startup_timeout_seconds`, `build_command`, `build_output_directory`, `enable_inspector`, `auto_reload` (superseded by the `servers` jsonb array; `build_servers()` synthesizes the old shape into the new for back-compat) |
| **CRUD/ACL** | `name`, `description`, `scope`, `team_id`, `owner_user_id`, `created_by`, `deleted` (soft-delete), timestamps |

There are **no** normalized child tables for run-config — everything is jsonb on
the one row, except the `plugin_repos` ⇄ `repository_plugin_repos` M:N join
(`0016_plugin_repos.sql`), which is the *only* existing precedent for "a repo
can reference other git repos."

### 1.2 Task → Repository is a mandatory 1:1 (FK by convention)

- `backend/src/task/models.rs:37` — `pub repository_id: String` (required,
  non-optional).
- `backend/migrations/0001_schema.sql:108-134` — `repository_id text NOT NULL`,
  plain btree index, **no DB foreign key**. `task_templates` is the only place
  with a real FK to `repositories`.
- Provisioning resolves it via a single lookup:
  `repos.find_repository_by_id(&task.repository_id)`
  (`backend/src/sandbox/k8s/auto_provision.rs:65`).

### 1.3 Provisioning is single-repo by construction

`backend/src/sandbox/models.rs:19-30`:

```rust
pub struct ProvisionPlan {
    pub repo_config: RepoConfig,     // ONE repo's config
    pub git_clone_url: String,       // ONE url
    pub git_branch: Option<String>,  // ONE branch
    pub git_provider: String,
    pub env_vars: Option<HashMap<String, String>>,
}
```

There is no `repos: Vec<…>` anywhere in the provision chain. The pod spec sets a
single `GIT_CLONE_URL` / `GIT_BRANCH` / `GIT_PROVIDER` env
(`backend/src/sandbox/k8s/pod_builder.rs:647-671`).

### 1.4 Servers are backend-built sidecar containers, keyed only by name

- `servers: Option<Vec<ServerConfig>>` already lets a repo declare **multiple**
  servers (`crates/api-schemas/src/repository.rs:83-107`). The frontend
  `servers-group.tsx` UI already edits an arbitrary list.
- Each server becomes a **sidecar container** built by the backend
  (`pod_builder.rs::build_server_containers`, `container_builder.rs` for Docker),
  not a process the sandbox runtime supervises. The start command is baked into
  the container's `command`/`args`, gated behind a `wait-for-.workspace-ready`
  sentinel, then `cd "$WORKSPACE_PATH" && <commands>`.
- **`ServerConfig` has no `repository_id` / repo-scoping field.** Every server
  shares one `WORKSPACE_PATH` (written to `/opt/moatless/data/server.env` by the
  sandbox at runtime, `sandbox/src/init/credentials.rs:203-209`). So today it is
  literally impossible to say "this preview server serves repo B."
- Ports: each `ServerConfig.port` is author-chosen (K8s) or Docker-dynamic;
  Traefik `IngressRoute` hostnames are `{task_id}--{port}.{proxy_domain}`
  (`k8s/ingress.rs:69`) — **already per-port, task-scoped, not repo-scoped**, so
  the routing layer needs no change for multi-repo.

### 1.5 The sandbox filesystem already tolerates 0 and n repos

- `sandbox/src/config.rs:266-275` — `repo_dir` falls back to `workspace_dir`
  when `GIT_CLONE_URL` is empty. **A zero-repo sandbox already boots**; nothing
  clones.
- `sandbox/src/files/git.rs::discover_git_repos()` walks *all* subdirs under
  `workspace_dir` for `.git`; `workspace_sync.rs` already snapshots the whole
  workspace design (per the persistence plan). The **read/diff/snapshot** side is
  multi-repo-tolerant; the **provisioning/config** side is strictly single-repo.
- The [`workspace-dir-multi-repo-persistence.md`](./workspace-dir-multi-repo-persistence.md)
  plan already splits `workspace_dir` (persisted container, cwd) from `repo_dir`
  (`/opt/moatless/workspace/<repo>`), and Phase 1 wording is live (it's the
  "Workspace layout" system-prompt note). **This proposal is the config-layer
  counterpart to that filesystem-layer plan.**

### 1.6 "Workspace" is already an overloaded word — and the backend already half-renamed it

The codebase uses "workspace" for **the per-task sandbox**. Crucially, the
backend Rust has **already largely renamed this to `Sandbox*`**:
`backend/src/sandbox/schemas.rs:132` is `SandboxStatusFullResponse` (with
`sandbox_state`, `sandbox_ready` fields), and `backend/src/sandbox/models.rs:110`
is `SandboxState`. The generated TS client reflects this — `sandboxStatusFullResponse.ts`
is the **live** type. The old `workspaceStatusFullResponse.ts` / `workspaceState.ts`
are **stale generated leftovers**: only re-exported by `model/index.ts`, never
imported by app code.

So the "workspace → sandbox" rename in the backend is roughly 80% done already.
What remains is (a) residual helper/function/doc names still saying "workspace"
when they mean sandbox, and (b) the stale generated TS artifacts. This is
resolved and specified in **§8** — and it's what frees the name **Workspace**
for the new config entity. The frontend deliberately **keeps** "workspace" as
the UI word for the *running* sandbox (see §7, §8).

### 1.7 Doc drift found during research (worth cleaning up)

- `CLAUDE.md` references a `drizzle/` migrations dir and `packages/types/` and
  `features/environments/` — **none exist**; migrations are
  `backend/migrations/*.sql`, the frontend feature is `features/repositories/`.
- `sandbox/CLAUDE.md` documents a `sandbox/src/server/` runtime module — **does
  not exist**; servers are backend-built containers.
- `packages/sandbox-schemas/server.schemas.ts` + `sandbox-api-contract/server.routes.ts`
  describe a dead "start/stop server over HTTP" model with no live consumer.
- `preview-environment-group.tsx` (single-server legacy UI) is in the tree but
  unrendered.

These are not blockers but the Workspace work is a natural moment to delete the
dead server-HTTP model and fix the docs.

---

## 2. Goals & non-goals

### Goals

1. A sandbox/task can run with **0 repositories** (blank scratch env), **1**
   (today's case, unchanged UX), or **n** (monorepo-of-repos, or
   frontend+backend).
2. **Repository** becomes a reusable *git pointer* — url, provider, default
   branch, credentials scope — reusable across many workspaces. It no longer
   describes *how it's run*.
3. **Workspace** owns *how it's run* — docker image, setup, env, servers,
   plugins, resources, system prompt — plus an ordered list of repo references
   (each with its own branch/clone options) and a set of servers.
4. A **Task** references a **Workspace**. Preserve the current single-repo
   creation flow as sugar over "workspace with one repo."
5. Preview servers can (optionally) be associated with a specific repo in the
   workspace, so `cd`-into-the-right-repo works with >1 repo.

### Non-goals

- Filesystem/persistence mechanics — owned by the persistence plan; this
  proposal *depends on* it (Phase 1 must be in place; Phase 2 makes multi-repo
  durable).
- Submodule / nested-repo semantics — top-level repos only.
- Per-task ad-hoc overrides of workspace config beyond what exists today
  (branch, message, model). Config lives on the Workspace.

---

## 3. Proposed model

### 3.1 Entities

```
Workspace (the run configuration + composition)
  ├─ id, name, description
  ├─ scope / team_id / owner_user_id      (same ACL model as repositories)
  ├─ kind: 'blank' | 'single' | 'multi'   (derived/validated, see §3.4)
  ├─ docker_image, docker_access, sandbox_service_account
  ├─ setup_commands, env_vars, system_prompt, plugins, resources
  ├─ WorkspaceRepo[]   (0..n ordered repo references)
  └─ ServerConfig[]    (0..n servers, each optionally bound to a WorkspaceRepo)

WorkspaceRepo (a repo's placement inside a workspace)
  ├─ workspace_id  → Workspace
  ├─ repository_id → Repository        (the reusable git pointer)
  ├─ branch: Option<String>            (override repo.default_branch)
  ├─ mount_name: Option<String>        (subdir under workspace_dir; defaults to repo name, disambiguates collisions — see persistence plan risk #4)
  ├─ clone_depth / clone_options
  ├─ is_primary: bool                  (the "main" repo for diff/PR/CLAUDE.md; exactly one when n>=1)
  └─ position: int                     (ordering)

Repository (now: pure git pointer, reusable)
  ├─ id, name, description
  ├─ kind: 'git' | 'template'          ('remote' folds into 'git'; 'template' may move to Workspace — see §6.3)
  ├─ remote_url, provider, default_branch
  └─ scope / team_id / owner_user_id
     (docker_image, setup_commands, servers, plugins, resources, system_prompt
      all REMOVED — they move to Workspace)

Task
  └─ workspace_id → Workspace          (replaces repository_id)
```

`ServerConfig` gains one optional field: `workspace_repo_id` (or `mount_name`)
so a server can declare which repo it serves. Null = workspace root (back-compat
/ blank workspaces / repo-agnostic servers).

### 3.2 Why a `WorkspaceRepo` join row (not a jsonb array of repos)

- Each placement carries per-workspace state (branch, mount name, primary flag,
  ordering) that shouldn't mutate the shared Repository.
- A Repository can be reused across workspaces (frontend repo in a "frontend
  only" workspace and in a "full stack" workspace).
- It mirrors the existing `repository_plugin_repos` M:N precedent, so the DAO
  pattern is familiar.

### 3.3 Servers move to the Workspace

`servers` (and the legacy flattened preview columns) move from `repositories` to
`workspaces`. Rationale: a server's start command (`bun run dev`) is a property
of *how this composition runs*, not of the git repo in the abstract — and with
multiple repos, "which repo's dev server" only makes sense at the workspace
level. The optional `workspace_repo_id` on each server says which checkout it
`cd`s into.

### 3.4 `kind` is derived, not authoritative

`blank` (0 repos) / `single` (1) / `multi` (n) is a convenience/validation label
derived from `WorkspaceRepo` count. Enforce: at most one `is_primary`; `single`
and `multi` require exactly one primary; `blank` requires zero repos and permits
servers bound only to the workspace root.

---

## 4. Provisioning changes

The single-repo `ProvisionPlan` (`backend/src/sandbox/models.rs`) becomes
multi-repo:

```rust
pub struct ProvisionPlan {
    pub workspace_config: WorkspaceConfig,   // was repo_config: RepoConfig
    pub repos: Vec<RepoCheckout>,            // 0..n; was git_clone_url/branch/provider
    pub env_vars: Option<HashMap<String, String>>,
}

pub struct RepoCheckout {
    pub clone_url: String,
    pub branch: Option<String>,
    pub provider: String,
    pub mount_name: String,      // subdir under workspace_dir
    pub is_primary: bool,
    pub depth: Option<u32>,
}

pub struct WorkspaceConfig {   // was RepoConfig, minus git identity
    pub name: String,
    pub docker_image: Option<String>,
    pub servers: Option<Vec<ServerConfig>>,   // ServerConfig gains mount_name
    pub plugin_ids: Option<Vec<String>>,
    pub system_prompt: Option<String>,
    pub code_execution: Option<serde_json::Value>,
    pub resources: Option<ResourceConfig>,
    pub plugin_repos: Option<Vec<String>>,
    pub docker_access, docker_networks, sandbox_service_account: …,
}
```

Sandbox-side (`sandbox/src/`):

- **Config** (`config.rs`): replace the single `git_clone_url`/`git_branch`/
  `repo_dir` with a `repos: Vec<RepoCheckout>` (passed via `REPO_CONFIG`/env,
  e.g. a `REPOS_JSON` env or extend `REPO_CONFIG`). `workspace_dir` stays cwd.
  `primary_repo_dir` = `workspace_dir/<primary mount_name>` (or `workspace_dir`
  when zero repos) — this is what today's `repo_dir` becomes, so all the
  repo-scoped ops from the persistence plan keep a single well-defined target.
- **Clone** (`init/git.rs`, `init/mod.rs`): loop over `repos`, clone each into
  its `mount_name` subdir. Zero repos → skip cloning entirely (already handled
  by the empty-URL fallback). Reuse the existing `.git`-exists skip per repo.
- **Server env** (`init/credentials.rs`, `server_env.rs`): instead of one
  `WORKSPACE_PATH`, resolve each server's `cd` target from its `mount_name`
  (default: primary repo, or workspace root if none). The server-env handoff
  file can carry per-server paths, or the backend can bake the resolved
  `mount_name` into each sidecar's `cd` (it knows mount names at plan time now —
  they're workspace config, not derived from the clone URL, removing the
  awkward runtime-injection dance in `credentials.rs:203-209`).
- **Diff/PR/CLAUDE.md** operations point at `primary_repo_dir`.

Backend pod/container builders (`pod_builder.rs`, `container_builder.rs`):

- Emit repo list instead of single `GIT_CLONE_URL`.
- `build_server_containers` sets each sidecar's `cd` from the server's
  `mount_name` (backend now knows it) — deterministic, no runtime override.
- Service/IngressRoute per server port — **unchanged** (already per-port).

Routing/preview URLs — **unchanged** (`{task_id}--{port}`).

---

## 5. Migration strategy (Repository → Workspace)

The safest path is **additive, then flip**, never a hard cut. Repositories keep
working throughout.

### Phase A — Introduce Workspace alongside Repository (no behavior change)

1. New tables: `workspaces`, `workspace_repos`. Reuse the repositories ACL
   columns/CHECK constraints verbatim (`scope`/`team_id`/`owner_user_id`).
2. `tasks` gains **`workspace_id text NULL`** (nullable during transition),
   keeping `repository_id` for now.
3. **Backfill:** for every existing repository, create a "shadow" workspace with
   one `workspace_repo` pointing at it, copying the run-config columns
   (docker_image, setup, env, servers, plugins, resources, system_prompt). Every
   existing task gets `workspace_id` = its repository's shadow workspace.
   - This is mechanical and reversible; a repo and its shadow workspace stay
     1:1 until someone edits them apart.
4. Provisioning reads Workspace when `task.workspace_id` is set, else falls back
   to the legacy repository path. New `ProvisionPlan` shape built from Workspace;
   a compatibility builder still handles legacy `repository_id`-only tasks.

### Phase B — Move creation flows onto Workspace

5. Task creation takes `workspaceId`. The "pick a repo" UX becomes sugar:
   selecting a bare repo silently creates/reuses a single-repo workspace
   (so the common case is unchanged for users). Add a "blank workspace" and
   "add another repo" affordance.
6. Repository edit UI **loses** the run-config sections (servers, setup, docker,
   plugins, system prompt) — those move to a Workspace edit surface. Repository
   edit keeps only git-pointer fields.
7. `.moatless/config.json` auto-import (`repository/config_reader.rs`) now
   populates a workspace (single-repo) rather than the repository row. Decide
   whether config lives in-repo (per §6.4).

### Phase C — Demote Repository, make `workspace_id` required

8. Drop the run-config columns from `repositories` (or leave them dormant and
   stop reading them). `repositories` = pure git pointer.
9. Make `tasks.workspace_id` `NOT NULL`; drop `tasks.repository_id` (after a
   release where both are written).
10. Delete dead code: server-HTTP schemas/routes, `preview-environment-group.tsx`,
    stale CLAUDE.md sections.

Each phase is independently shippable and DB-migration-gated. Phase A is pure
addition + backfill (low risk); Phase C is the only destructive step and only
after both paths have run in production.

---

## 6. Interaction with existing systems / open questions

### 6.1 Dependency on the persistence plan

Multi-repo **durability** requires Phase 2 of
[`workspace-dir-multi-repo-persistence.md`](./workspace-dir-multi-repo-persistence.md)
(whole-workspace S3 sync). Without it, only the primary repo persists across
restarts and secondary repos are re-cloned. Order: persistence Phase 1 (done) →
this Phase A/B can proceed for config → persistence Phase 2 before advertising
durable multi-repo → this Phase C.

### 6.2 Mount-name collisions

Two repos whose URL last-segment matches (e.g. two `api` repos) collide on
`mount_name`. `WorkspaceRepo.mount_name` is explicit and user-editable, with a
uniqueness constraint per workspace, resolving the persistence plan's open
risk #4.

### 6.3 Templates

`RepositoryKind::Template` today means "a starting-point repo." With workspaces,
"template" is better modeled as a **Workspace template** (a pre-baked
composition of repos + servers). Decide whether `kind: template` stays on
Repository, moves to Workspace, or both exist. Leaning: Workspace templates
supersede repository templates; `task_templates` (which already FK to
repositories) would re-point at workspaces.

### 6.4 Where does config live — DB or in-repo `.moatless/config.json`?

Today config auto-imports from a repo's `.moatless/config.json`. With multiple
repos, whose config wins? Options: (a) only the primary repo's config seeds the
workspace; (b) workspace config is authored in the DB/UI and in-repo config is
just a seed for single-repo sugar. Recommend (b) as the model, (a) as the
import convenience.

### 6.5 Credentials with multiple providers

`RepoCheckout` carries its own provider; `GetCredentials` (gRPC) must resolve a
token per repo/provider rather than one. The provider-token machinery already
keys by provider, so this is a loop, not a redesign — but worth an explicit
per-repo credential-fetch pass in `init/credentials.rs`.

### 6.6 Zero-repo sandboxes

Already boot (`config.rs` fallback). Needs: task-creation allows a blank
workspace; the "Workspace layout" prompt note adapts ("no repository checked
out; the workspace dir is your scratch area"); diff/PR features degrade
gracefully (nothing to diff) or bind to whatever the agent later clones.

### 6.7 Naming — DECIDED (see §8)

**Decision:** reserve **Workspace** for the new *config* entity. In the
**backend**, rename the residual per-task "workspace"-named symbols (which
actually mean *sandbox*) to **sandbox**. In the **frontend**, keep **Workspace**
as the *UI* term for the **running** sandbox (the "workspace" the user looks at
= the live sandbox). Full plan in §8.

---

## 7. Frontend changes (sketch)

**Vocabulary rule (see §8):** in the frontend, **"Workspace" keeps its existing
meaning as the UI term for the *running* sandbox** — the live thing the user
looks at in the right-hand panel (`WorkspacePanel`, `workspace-header.tsx`, the
Preview/Files/Sandbox tabs). The **new config entity** is surfaced under a
distinct label to avoid confusion — recommend **"Workspace config"** or
**"Environment"** in config/admin surfaces, while task-runtime UI keeps saying
"workspace" for the sandbox. (Exact user-facing label is a copy decision; the
important invariant is: *running thing = workspace (UI) = sandbox (backend);
config thing = the new entity*.)

- New `features/workspaces/` (config CRUD): general settings, repos list
  (add/remove/reorder repos, set primary, per-repo branch + mount name),
  servers (now with an optional "serves repo" selector), code execution,
  plugins, agent config. Most sections are lifted almost verbatim from
  `features/repositories/components/sections/` — they already exist.
- `features/repositories/` shrinks to git-pointer CRUD (url, provider, branch).
- Task creation: repo picker → workspace picker, with "quick single-repo"
  default that hides the workspace concept for the common case, plus "blank" and
  "multi-repo" paths.
- Query keys: add a `workspaces` namespace (repositories keys today are flat and
  un-namespaced — good moment to normalize, see `query-keys.ts`).
- Preview panel (`features/preview/`, `workspace-panel.tsx`): server tabs may
  now group by repo; `ServerEntry` already flows from status — add repo grouping
  when >1 repo.

---

## 8. Naming resolution: backend "workspace" → "sandbox"; frontend keeps "workspace" as UI term

To free the name **Workspace** for the new config entity (§3) without a
confusing collision, we split the vocabulary by layer:

| Layer | The *running* per-task thing | The new *config* thing |
|---|---|---|
| **Backend (Rust) identifiers** | **`sandbox`** (already the norm — `SandboxStatusFullResponse`, `SandboxState`) | **`workspace`** (`Workspace`, `WorkspaceRepo`, `workspaces` table) |
| **API / generated TS types** | `Sandbox*` (already live) | `Workspace*` config DTOs |
| **Frontend UI copy & components** | **"Workspace"** (kept — `WorkspacePanel`, the live sandbox the user views) | "Workspace config" / "Environment" label |

This is intentionally asymmetric and matches how people actually talk: the user
*sees* a "workspace" (the running sandbox); the thing they *configure* is the new
Workspace entity in the backend/API. The frontend just doesn't expose the word
"sandbox" prominently — it keeps calling the live view "workspace."

### 10.1 Backend rename (mostly cleanup — the core types are already `Sandbox*`)

Already done (no action): `SandboxStatusFullResponse`, `SandboxState`,
`sandbox_state`, `sandbox_ready` (`backend/src/sandbox/schemas.rs`,
`models.rs`).

Residual identifiers still saying "workspace" but meaning sandbox — rename to
`sandbox` as part of this work (all internal, no API surface):

- `backend/src/sandbox/grpc_server.rs:95` `workspace_key(...)` → `sandbox_key`
- `backend/src/sandbox/docker/naming.rs:60` `generate_workspace_host_path` → `generate_sandbox_host_path`
- `backend/src/sandbox/k8s/pod_builder.rs:364` `resolve_workspace_size` → `resolve_sandbox_size`
- `backend/src/sandbox/k8s/pod_builder.rs:374` `node_workspace_host_path` → `node_sandbox_host_path`
- `backend/src/sandbox/k8s/pod_builder.rs:405` `build_node_workspace_setup_container` → `..._sandbox_setup_..` (and the `node-workspace-setup` init-container name)
- `backend/src/sandbox/k8s/provider.rs:538` `deployment_has_workspace_host_path`, `:734` `delete_task_workspace` → `..._sandbox_...`
- `backend/src/sandbox/k8s/storage.rs:76` `syncs_workspace` → `syncs_sandbox` (semantic check)
- `backend/src/sandbox/CLAUDE.md` — the stale `WorkspaceStatusFullResponse` row and any "workspace" prose that means sandbox.

**Deliberately NOT renamed** — these are the genuine *filesystem* "workspace"
(the persisted directory, per the persistence plan), not the sandbox entity, and
must stay:

- `workspace_dir` / `WORKSPACE_DIR` / `WORKSPACE_PATH` (the `/opt/moatless/workspace` path)
- the `workspace-data` PVC/volume name, `workspace_sync.rs`, `node-workspace-setup`'s
  chown target — anything referring to the on-disk workspace directory.

This keeps a clean three-way distinction: **sandbox** (the pod/runtime),
**workspace dir** (the persisted directory on disk), **Workspace** (the new
config entity). The first two are unrelated to the config entity; only the name
"Workspace" (capitalized config noun) is new.

### 10.2 Frontend: keep "workspace" as the running-sandbox UI term

- Keep `WorkspacePanel`, `workspace-header.tsx`, "workspace" copy for the live
  view — no rename. Optionally delete the **stale generated** `workspaceStatusFullResponse.ts`
  / `workspaceState.ts` (regenerate orval so `model/index.ts` stops re-exporting
  them) since app code already uses `Sandbox*`.
- The new config entity's frontend feature (`features/workspaces/`, §7) uses a
  distinct label ("Workspace config"/"Environment") in admin/config surfaces so
  users don't confuse "configure my Workspace" with "look at the running
  workspace."

### 10.3 Sequencing

Do §10.1 backend residual renames as a small, standalone refactor **before**
Phase A (§5) introduces the `Workspace` config type — otherwise a new
`Workspace` struct would sit next to old `workspace_*` sandbox helpers and
re-muddy the vocabulary. It's a low-risk, mechanical, internal-only rename (no
API/DB change) and makes the subsequent Workspace-entity code unambiguous.

## 9. Risk summary

| Risk | Mitigation |
|---|---|
| Big blast radius (touches DB, backend provisioning, sandbox runtime, frontend) | Strictly phased; Phase A is additive + backfill; only Phase C is destructive |
| "Workspace" naming collision with per-task sandbox vocabulary | Decide §6.7 up front before any code |
| Multi-repo not durable without persistence Phase 2 | Sequence behind that plan; gate the "durable multi-repo" claim |
| Secondary-repo credentials / provider mix | Per-repo credential loop (§6.5) |
| Mount-name collisions | Explicit editable `mount_name` + per-workspace uniqueness |
| Template semantics ambiguity | Resolve §6.3 before Phase B |
| Dead code / doc drift discovered (§1.7) | Clean up during Phase C |

---

## 10. Recommended next step

1. **Decide naming** (§6.7) — Workspace-as-config vs. rename per-task usage.
2. Ratify the entity model (§3) and the additive migration (Phase A) as a
   concrete spec under `specs/workspaces/` mirroring `specs/repositories/`.
3. Land persistence-plan Phase 2 (durable multi-repo) in parallel — it's the
   only hard blocker for advertising >1 repo.
4. Prototype Phase A behind a flag: tables + backfill + provisioning that reads
   Workspace when present, verifying the single-repo path is byte-for-byte
   unchanged before touching creation UX.
