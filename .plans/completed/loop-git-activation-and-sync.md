# Make Loop execution an explicit administrator decision

> **Status:** shipped
> **Updated:** 2026-08-01
> **Vocabulary:** CONTEXT.md

## Current solution

A repository can currently grant itself recurring execution authority.

`LoopFrontmatter` accepts `active` and `run_as_user_id` from `.moatless/loops/<name>/LOOP.md`.

Missing `active` defaults to `true`.

Missing `run_as_user_id` falls back through the platform default, Repository owner, and Repository creator.

| Hop | Current behavior | Evidence |
|---|---|---|
| Parse Git configuration | `LoopFrontmatter.active` and `LoopFrontmatter.run_as_user_id` are accepted as ordinary Git-owned fields. | `backend/src/loops/sync/models.rs:13`, `backend/src/loops/sync/models.rs:28`, `backend/src/loops/sync/models.rs:70` |
| Resolve execution authority | `resolve_loop` uses `active.unwrap_or(true)` and resolves run-as from frontmatter, the platform default, Repository owner, or Repository creator. | `backend/src/loops/sync/mod.rs:161`, `backend/src/loops/sync/mod.rs:168`, `backend/src/loops/sync/mod.rs:199`, `backend/src/loops/sync/mod.rs:510` |
| Persist a Git Loop | `upsert_managed_loop` writes `triggers.active` and `trigger_rules.run_as_user_id` from the resolved Git declaration. | `backend/src/loops/sync/mod.rs:421`, `backend/src/loops/sync/mod.rs:454`, `backend/src/loops/store.rs:814`, `backend/src/loops/store.rs:830`, `backend/src/loops/store.rs:897` |
| Reconcile automatically | Repository creation invokes Loop sync, and Task start invokes the shared Repository configuration sync before provisioning. | `backend/src/repository/handlers.rs:110`, `backend/src/task/handlers.rs:1097`, `backend/src/task/handlers.rs:1108`, `backend/src/repository/handlers.rs:1127` |
| Admit firing | Subscription lookups and Schedule claims filter only on `t.active = true`. | `backend/src/loops/store.rs:599`, `backend/src/loops/store.rs:615`, `backend/src/loops/store.rs:629`, `backend/src/loops/store.rs:641`, `backend/src/loops/store.rs:932` |
| Choose task identity | `LoopContext::effective_user_id` falls back to `created_by`; Schedule execution repeats the same fallback chain. | `backend/src/adapters/models.rs:66`, `backend/src/loops/firing/schedule_runner.rs:103`, `backend/src/loops/firing/inbound.rs:88` |
| Mutate through HTTP | Create, update, and delete use Scope-based visibility/control; they do not require the caller to be an administrator. | `backend/src/loops/http/handlers.rs:123`, `backend/src/loops/http/handlers.rs:251`, `backend/src/loops/http/handlers.rs:475`, `backend/src/loops/store.rs:450`, `backend/src/loops/store.rs:566` |

The existing Git provenance model is incomplete for overrides.

`triggers.managed_by = 'repo_loop'` identifies Git reconciliation.

`triggers.source_repository_id` and `triggers.loop_name` form the stable Git key.

The public `Loop` response does not expose those fields.

There is no configuration-source field, source path, override state, or deleted tombstone.

| Current persistence | Meaning | Limitation |
|---|---|---|
| `triggers.managed_by IS NULL` | Manual/API-authored Loop. | It cannot distinguish a plain manual Loop from an overridden Git Loop. |
| `triggers.managed_by = 'repo_loop'` | Loop currently reconciled from Git. | Changing this value would lose the only configuration-source signal. |
| `triggers.source_repository_id` | Repository that declared the Loop. | It is not returned by the HTTP response. |
| `triggers.loop_name` | Directory name and reconciliation key. | It is not named as general provenance and is excluded from manual rows by the current partial unique index. |
| No deletion marker | Delete cascades the Loop rows. | A later sync recreates a deleted Git Loop. |

The current sync failure behavior is neither atomic nor deletion-safe.

Each `upsert_managed_loop` commits its own transaction.

`sync_loops` processes Loops sequentially.

A database failure after earlier upserts leaves a partial repository result.

Malformed files are removed by `filter_map` before the set of present directory names is built.

An existing Loop can therefore be treated as absent and deleted when its file is present but malformed.

`LoopStore::update` also updates `triggers` before opening the transaction used for `trigger_rules` and firing-condition rows.

| Failure | Current state after failure | Evidence |
|---|---|---|
| One `LOOP.md` fails parsing | The file is absent from `parsed`; its prior Git Loop can enter the deletion set. | `backend/src/loops/sync/mod.rs:63`, `backend/src/loops/sync/mod.rs:81`, `backend/src/loops/sync/mod.rs:107` |
| A later Loop upsert fails | Earlier Loop transactions remain committed. | `backend/src/loops/sync/mod.rs:86`, `backend/src/loops/store.rs:818`, `backend/src/loops/store.rs:921` |
| Generic update fails after changing `triggers` | `name`, `active`, Scope, and owner may be committed while config/source updates fail. | `backend/src/loops/store.rs:450`, `backend/src/loops/store.rs:472`, `backend/src/loops/store.rs:486` |
| Git fetch fails during explicit Repository sync | The Repository handler replaces the Loop result with an empty summary, making “nothing declared” indistinguishable from “sync did not run.” | `backend/src/repository/handlers.rs:1139` |

The frontend already has an administrator-only route tree for Loops.

The Loop detail page exposes a direct active toggle.

The edit form accepts a raw Run As User ID and edits Git-managed Loops without provenance awareness.

Workspace UI contains the prior art for Git source banners, override, reset, deleted tombstones, and read-only forms.

| UI surface | Current behavior | Prior art or gap |
|---|---|---|
| Loop sidebar | Shows active/off and firing kind. | No Git/manual, overridden, deleted, awaiting-approval, or paused state. `apps/frontend/src/features/loops/components/admin-loops-sidebar.tsx:35` |
| Loop detail | Toggles `active` through generic update. | No atomic run-as selection or activation lifecycle. `apps/frontend/src/routes/admin/loops/$loop-id/index.lazy.tsx:164` |
| Loop edit | Edits all config and sends `active` plus `runAsUserId`. | No read-only Git state or override/reset action. `apps/frontend/src/features/loops/components/loop-form.tsx:295`, `apps/frontend/src/features/loops/components/loop-form.tsx:535`, `apps/frontend/src/features/loops/components/loop-form.tsx:593` |
| Workspace edit | Shows Git provenance, disables editing, supports Override to edit, Reset to Git, Restore from Git, and tombstones. | Pattern to adapt for Loop configuration authority. `apps/frontend/src/features/workspaces/components/workspace-git-source-banner.tsx:15` |
| Repository edit | Explicit `Sync from Git` can reconcile Loops and Workspaces. | Warning only describes reclaimed Workspace overrides. `apps/frontend/src/features/repositories/components/sections/repository-config-edit-section.tsx:192` |

## Planned solution

Git defines Loop configuration.

Git never supplies Loop execution authority.

Every Loop mutation requires an administrator.

Every runnable Loop stores an explicit run-as user.

Configuration authority and execution approval are independent axes.

### Problem

A change merged into a Repository can currently create an active recurring workload and choose the identity attributed to its Tasks.

An ordinary user can currently mutate a Loop through the HTTP/CLI surface when Scope grants control.

An administrator cannot locally override a Git Loop without losing future provenance or being overwritten by the next sync.

| Actor | Problem to solve |
|---|---|
| Administrator | Must explicitly approve who a Loop runs as before it can fire. |
| Repository author | May describe recurring work but must not grant execution authority. |
| Ordinary user | May read a Loop allowed by Scope but must not mutate any Loop. |
| Operator | Must be able to distinguish Git-synced, overridden, deleted, awaiting-approval, paused, and active state. |
| Implementer | Needs one transactional lifecycle seam instead of independent `active`, run-as, config-source, and delete writes. |

### Behaviour

#### Administrator imports or re-syncs Repository Loops

| Given | When | Then |
|---|---|---|
| A Repository contains a valid new `.moatless/loops/<name>/LOOP.md`. | An administrator invokes Repository configuration sync from the UI or `moat repos sync-config`. | The Loop is created with Git configuration authority, `active = false`, and `run_as_user_id = NULL`. |
| The Git file contains `active` or `runAsUserId`. | The administrator syncs. | Those values do not affect stored execution state; the result contains warnings naming the ignored fields. |
| A Git-synced Loop is active or paused. | The administrator re-syncs, even when Git content is unchanged. | The Git-owned configuration is replaced, `active` becomes `false`, `run_as_user_id` becomes `NULL`, and the result counts one revoked approval. |
| A Git-synced Loop no longer exists in the fetched directory snapshot. | The administrator syncs. | The Loop is removed after all fetched directories have been classified successfully enough to confirm absence. |
| An overridden Loop or deleted override shares a stable key with a Git declaration. | The administrator syncs the Repository. | The local row is skipped and reported; Repository-wide sync never reclaims it. |
| A Repository is created or a Task starts. | Normal Repository/Task behavior runs. | Loop synchronization does not run. Existing automatic Workspace reconciliation remains unchanged. |
| A non-admin invokes Repository configuration sync. | Authorization runs before a Git fetch or write. | The request is rejected as forbidden and no Repository configuration is changed. |

#### Administrator approves, pauses, and resumes execution

| Given | When | Then |
|---|---|---|
| A Loop has no run-as user and is inactive. | An administrator activates it with an existing user or Bot user. | User validation, `run_as_user_id` assignment, and `active = true` commit atomically. |
| A Loop is active with an explicit user. | An administrator pauses it. | `active` becomes `false`; the selected run-as user remains. |
| A Loop is paused with a valid run-as user. | An administrator resumes it. | `active` becomes `true`; no new user selection is required. |
| A Loop is awaiting approval. | An administrator tries to resume without selecting a user. | The request is rejected and the Loop remains inactive. |
| A Loop is active. | A Schedule becomes due or a matching Subscription event arrives. | The Task is attributed to the stored run-as user; no creator fallback is used. |
| A Loop is active but its stored user can no longer be resolved. | A firing path evaluates it. | No Task is created; the failure is logged and the admin UI identifies the invalid run-as user as the repair needed. |
| A non-admin invokes create, update, delete, activate, pause, resume, override, reset, restore, or Repository sync. | The handler checks the role. | The request is forbidden before any state change. |

#### Administrator overrides and returns to Git

| Given | When | Then |
|---|---|---|
| A Loop is Git-synced. | An administrator selects Override to edit without changing fields. | Configuration source becomes manual; Git provenance remains; active/paused state and run-as user remain unchanged. |
| A Loop is Git-synced. | An administrator sends a generic config update without overriding. | The request is rejected with guidance to override first. |
| A Loop is overridden. | Repository configuration sync runs. | The Loop is skipped and its config plus execution approval remain unchanged. |
| An overridden Loop is still declared in Git and resolves successfully. | An administrator selects Reset to Git. | Only that Loop is replaced from Git, configuration source becomes Git, and execution returns to inactive/awaiting approval. |
| An overridden Loop is absent or invalid in Git. | An administrator selects Reset to Git. | The override remains unchanged and the response explains why reset did not take effect. |
| A Git-origin Loop is deleted, whether currently Git-synced or overridden. | An administrator confirms delete. | The row becomes a deleted manual override tombstone, keeps provenance, clears execution approval, and remains available to administrators for restore. |
| A deleted override is still declared and valid in Git. | An administrator selects Restore from Git. | Only that Loop is restored as Git-synced, inactive, and awaiting approval. |
| A deleted override is absent or invalid in Git. | An administrator selects Restore from Git. | The tombstone remains and the response explains why restore did not take effect. |
| A manual Loop with no Git provenance is deleted. | An administrator confirms delete. | The Loop is hard-deleted through the existing cascade. |

#### Administrator authors a manual Loop

| Given | When | Then |
|---|---|---|
| An administrator creates a manual Loop without a run-as user. | Create succeeds. | The Loop defaults inactive and awaits approval. |
| An administrator creates a manual Loop with `active = true`. | Request validation runs. | A valid explicit run-as user is required in the same request. |
| A manual Loop is active or paused. | An administrator edits non-lifecycle configuration. | Execution state remains unchanged unless a lifecycle endpoint is invoked. |
| A user-scoped manual Loop was created by an ordinary user before this change. | The change is deployed. | Scope and ownership remain for reads; future mutations require an administrator. |

#### Deployment migration

| Given | When | Then |
|---|---|---|
| An existing Loop has `managed_by = 'repo_loop'`. | Migration runs. | It becomes Git-synced, provenance is retained, `active` becomes `false`, and `run_as_user_id` is cleared. |
| Any existing manual Loop has no `run_as_user_id`. | Migration runs. | It becomes inactive; no creator identity is backfilled. |
| An existing manual Loop has an explicit `run_as_user_id`. | Migration runs. | Its active or paused state is preserved. |
| Existing API clients use `/api/v1/triggers`. | Routes are upgraded. | The alias retains the same read and mutation authorization/lifecycle behavior as `/api/v1/loops`. |

### Architecture and key flows

This section is included because sync, persistence, Repository orchestration, HTTP, CLI, frontend, and two firing paths change responsibility.

The deep module is the Loop store/lifecycle seam.

Callers describe a lifecycle transition or a complete Repository reconciliation plan.

The store owns the transaction, row locking, invariants, and result counts.

| Module | Observed responsibility | Proposed responsibility and change | Evidence |
|---|---|---|---|
| `backend/src/loops/sync/fetcher.rs` | Lists directories and fetches files from GitHub without using `Repository.default_branch`. | Fetch one complete snapshot from the Repository's configured branch; return every directory name and either file content or a per-directory fetch classification so “present but invalid” cannot become “absent.” | `backend/src/loops/sync/fetcher.rs:20`, `backend/src/loops/sync/fetcher.rs:63`, `backend/src/loops/sync/fetcher.rs:84` |
| `backend/src/loops/sync/parser.rs` and `models.rs` | Parse authority fields as ordinary frontmatter and return `None` on malformed input. | Return a classified parse result keyed by directory; retain `active`/`runAsUserId` only to produce ignored-authority warnings, never as resolved values. | `backend/src/loops/sync/parser.rs:11`, `backend/src/loops/sync/models.rs:13` |
| `backend/src/loops/sync/mod.rs` | Resolves and writes one Loop at a time; computes deletions from successfully parsed names. | Build a side-effect-free `RepositoryLoopSyncPlan`: valid desired Loops, skipped present keys, confirmed absent keys, and warnings; pass the complete plan to one store transaction. Remove run-as fallback and automatic callers. | `backend/src/loops/sync/mod.rs:39`, `backend/src/loops/sync/mod.rs:75`, `backend/src/loops/sync/mod.rs:161`, `backend/src/loops/sync/mod.rs:421` |
| `backend/src/loops/store.rs` | Owns Loop SQL, but generic update splits one logical write across transactions and managed sync commits per Loop. | Own atomic create/update/delete/lifecycle methods and one repository reconciliation transaction. Enforce config-source, tombstone, active/run-as, row-locking, skip, and count invariants behind a small interface. | `backend/src/loops/store.rs:1`, `backend/src/loops/store.rs:450`, `backend/src/loops/store.rs:792` |
| `backend/src/loops/http/handlers.rs` and `http/mod.rs` | Expose scoped CRUD and recent Tasks. | Require admin for every mutation; add lifecycle routes; keep scoped reads; reject edits to Git-synced config; map stable conflict/forbidden/not-found/reset-failed results. Register the same lifecycle aliases under deprecated `/api/v1/triggers`. | `backend/src/loops/http/handlers.rs:87`, `backend/src/loops/http/mod.rs:14` |
| `backend/src/repository/handlers.rs` | Shared `sync_repo_config` always invokes Loop sync and Workspace sync; callers differ only by Workspace `SyncForce`. | Split explicit administrator Repository sync from automatic Workspace-only reconciliation. Explicit sync reconciles Loops while continuing current Workspace force behavior. Repository create and Task start call Workspace sync only. | `backend/src/repository/handlers.rs:1077`, `backend/src/repository/handlers.rs:1127`, `backend/src/task/handlers.rs:1108` |
| `backend/src/loops/firing/*`, `backend/src/adapters/models.rs`, and runnable queries in `LoopStore` | Filter on active and fall back to creator identity. | Make the runnable read shape require a resolved run-as user. Schedule and Subscription admission require active, non-deleted, explicit-user Loops. Remove `effective_user_id` fallback. | `backend/src/loops/store.rs:599`, `backend/src/loops/store.rs:930`, `backend/src/adapters/models.rs:66`, `backend/src/loops/firing/schedule_runner.rs:103`, `backend/src/loops/firing/inbound.rs:88` |
| `crates/api-schemas/src/loops.rs` | Exposes firing `source`, active, config, and basic sync counts. | Add configuration provenance, derived execution state, lifecycle requests/responses, warnings, revocation/skip counts, and reset result. Keep `source` as the Schedule/Subscription firing condition; use `configSource` for Git/manual authority. | `crates/api-schemas/src/loops.rs:47`, `crates/api-schemas/src/loops.rs:183`, `crates/api-schemas/src/loops.rs:210`, `crates/api-schemas/src/loops.rs:274` |
| `crates/cli/src/commands/loops.rs` and `repos.rs` | Generic Loop update mutates active/run-as; Repository sync reports basic counts. | Add explicit lifecycle commands and status output; make Git provenance and execution state readable; show revoked approvals, skipped overrides/tombstones, and ignored Git authority warnings. | `crates/cli/src/commands/loops.rs:20`, `crates/cli/src/commands/loops.rs:126`, `crates/cli/src/commands/repos.rs:63` |
| `apps/frontend/src/features/loops/**` and admin Loop routes | Provide CRUD form, raw user ID, active switch, sidebar, and recent Tasks. | Add Git provenance banner, named admin user picker, execution panel, lifecycle mutations, read-only Git form, tombstone restore, status badges, and tests. | `apps/frontend/src/features/loops/components/loop-form.tsx:247`, `apps/frontend/src/features/loops/components/admin-loops-sidebar.tsx:35`, `apps/frontend/src/routes/admin/loops/$loop-id/index.lazy.tsx:164` |
| `apps/frontend/src/features/repositories/components/sections/repository-config-edit-section.tsx` | Runs explicit Repository sync and warns about reclaimed Workspace overrides. | Require confirmation that every Git-synced Loop approval will be revoked; report Loop outcomes while retaining the existing Workspace reclaim warning. | `apps/frontend/src/features/repositories/components/sections/repository-config-edit-section.tsx:192` |

The proposed internal interfaces are implementation boundaries, not external promises.

They may be renamed if the same depth and invariants remain.

```rust
impl LoopStore {
    async fn reconcile_repository_loops(
        &self,
        repository_id: &str,
        plan: RepositoryLoopSyncPlan,
    ) -> Result<LoopSyncSummary, LoopStoreError>;

    async fn activate(
        &self,
        loop_id: &str,
        run_as_user_id: &str,
    ) -> Result<LoopRecord, LoopLifecycleError>;

    async fn pause(&self, loop_id: &str) -> Result<LoopRecord, LoopLifecycleError>;
    async fn resume(&self, loop_id: &str) -> Result<LoopRecord, LoopLifecycleError>;
    async fn override_from_git(
        &self,
        loop_id: &str,
        patch: Option<UpdateLoopRequest>,
    ) -> Result<LoopRecord, LoopLifecycleError>;
    async fn apply_git_reset(
        &self,
        loop_id: &str,
        desired: DesiredGitLoop,
    ) -> Result<LoopRecord, LoopLifecycleError>;
}
```

No new trait is required.

`LoopStore` already has multiple production and test consumers and already owns all four persistence tables.

#### Key flow: explicit Repository sync

This flow is included because ordering, partial logical success, transaction boundaries, execution revocation, and authorization decide whether the security guarantee holds.

1. `POST /api/v1/repositories/{repository_id}/sync-config` authenticates and rejects a non-admin before remote access.
2. The Repository is loaded through Scope-aware read access.
3. Workspace sync continues with `SyncForce::All`, preserving current Workspace behavior.
4. Loop fetch resolves the Repository's configured branch and captures the `.moatless/loops/` directory snapshot.
5. Every directory is classified as valid, present-but-skipped, or fetch-failed.
6. A whole-fetch failure returns an error and performs no Loop database work.
7. Parse and resolution produce valid desired Loops, per-key skips, and warnings.
8. Tag resolution that can create rows participates in the same database transaction as the Loop plan; it must not leave tag side effects after Loop rollback.
9. The transaction takes a Repository-scoped advisory transaction lock, then locks existing provenance rows.
10. Valid Git-controlled rows are inserted or replaced with `config_source = 'git'`, `deleted = false`, `active = false`, and `run_as_user_id = NULL`.
11. Valid keys matching `config_source = 'manual'` provenance rows are skipped, whether live overrides or deleted tombstones.
12. Existing Git-controlled keys confirmed absent are deleted through the current cascade.
13. Present-but-skipped keys are preserved exactly, including execution approval.
14. The transaction commits all valid changes and confirmed removals or rolls them all back.
15. The response distinguishes created, updated, removed, approvals revoked, overrides/tombstones skipped, invalid files skipped, and ignored-authority warnings.

#### Key flow: lifecycle transition concurrent with sync

This flow is included because an activation racing a re-sync must have deterministic authority.

1. Lifecycle handlers require administrator role and validate the requested transition.
2. A Git-origin Loop lifecycle transaction uses the same Repository-scoped advisory transaction lock as reconciliation.
3. The target row is selected `FOR UPDATE`.
4. If activation commits before re-sync, the later re-sync revokes it.
5. If re-sync commits before activation, the later activation approves the newly synced configuration.
6. Commit order is the observable order; no merged or half-approved state exists.

#### Key flow: per-Loop reset or restore

This flow is included because it crosses Git, override state, and execution approval without touching sibling Loops.

1. The administrator invokes `reset-git` for an overridden or deleted Git-origin Loop.
2. The backend reads retained `source_repository_id`, `source_key`, and `source_config_path`.
3. The backend fetches and resolves only that stable key from the configured Repository branch.
4. Missing, malformed, or unresolved Git configuration returns `resynced = false` and leaves the override/tombstone unchanged.
5. A valid desired Loop is applied in one locked transaction.
6. The row returns to `config_source = 'git'`, `deleted = false`, `active = false`, and `run_as_user_id = NULL`.
7. Sibling Git Loops are not reconciled.

### Data model

This section is included because configuration provenance, deletion, execution approval, lifecycle, consistency, and migration all change durable state.

#### Logical model

Configuration authority and execution approval are independent.

| Axis | State | Meaning | Allowed transitions |
|---|---|---|---|
| Configuration authority | Git-synced | Git is authoritative; UI/CLI config edit is read-only. | Override, delete, Repository re-sync. |
| Configuration authority | Manual | No Git provenance; locally authored configuration is authoritative. | Edit, delete. |
| Configuration authority | Overridden | `config_source = 'manual'` with Git provenance and `deleted = false`. | Edit, reset to Git, delete. |
| Configuration authority | Deleted override | `config_source = 'manual'` with Git provenance and `deleted = true`. | Restore from Git. |
| Execution approval | Awaiting approval | `run_as_user_id IS NULL` and `active = false`. | Activate with explicit user. |
| Execution approval | Active | `run_as_user_id IS NOT NULL` and `active = true`. | Pause, change user through activate, Git re-sync/reset, delete. |
| Execution approval | Paused | `run_as_user_id IS NOT NULL` and `active = false`. | Resume, change user through activate, Git re-sync/reset, delete. |

The following invariants are mandatory.

| Invariant | Enforcement |
|---|---|
| `active = true` requires a present, valid `run_as_user_id`. | Lifecycle/create validation, atomic writes, migration, and runnable SQL filters. |
| Awaiting approval is always inactive. | Import, re-sync, reset, restore, create defaults, and migration. |
| A deleted override is always inactive and unapproved. | Tombstone transaction clears `active` and `run_as_user_id`. |
| Git import/re-sync/reset/restore never preserves execution approval. | Reconciliation and reset write execution fields as local authority defaults. |
| Override preserves execution approval. | Override transaction changes configuration source and optional config fields only. |
| Repository sync never changes a live override or tombstone. | Reconciliation key lookup skips every manual row with retained Git provenance. |
| A malformed or unresolved present key is not absent. | Fetch/parse plan retains every directory key and deletion uses confirmed absence only. |
| One Git provenance key maps to one durable Loop across Git, override, and tombstone states. | Unique index is independent of `config_source`. |

#### Physical representation

Add migration `backend/migrations/0112_loop_configuration_authority.sql`.

The exact migration number should remain the next free number if another migration lands first.

Replace the legacy reconciliation discriminator with Workspace-shaped provenance.

| Table/field | Physical change | Meaning |
|---|---|---|
| `triggers.config_source text NOT NULL` | Add with check constraint `IN ('git', 'manual')`; backfill `git` when `managed_by = 'repo_loop'`, otherwise `manual`. | Current configuration authority. API field: `configSource`. |
| `triggers.source_repository_id text` | Reuse existing column. | Repository that originally declared the Loop; retained after override/delete. |
| `triggers.source_key text` | Rename existing `loop_name`. | Stable `.moatless/loops/<source_key>/` directory name; retained after override/delete. |
| `triggers.source_config_path text` | Add nullable. | `.moatless/loops/<source_key>/LOOP.md`; retained after override/delete. |
| `triggers.deleted boolean NOT NULL DEFAULT false` | Add. | True only for a Git-origin deleted override tombstone. |
| `triggers.managed_by` | Drop after backfill and code migration. | Replaced by `config_source`; retaining both would permit disagreement. |
| `trigger_rules.run_as_user_id` | Keep nullable. | Null means awaiting approval; non-null is the explicit execution identity. |
| `triggers.active` | Keep non-null current representation. | False with user is paused; false without user is awaiting approval. |

Replace `triggers_loop_idx`.

The new unique index covers `(source_repository_id, source_key)` whenever both are non-null.

It must include Git-synced rows, live overrides, and deleted tombstones.

Plain manual Loops keep both fields null and do not participate.

Migration execution-state updates occur in the same migration transaction.

```sql
-- Conceptual order, not copy-ready migration SQL.
UPDATE triggers
SET active = false
WHERE config_source = 'git';

UPDATE trigger_rules r
SET run_as_user_id = NULL
FROM triggers t
WHERE r.trigger_id = t.id
  AND t.config_source = 'git';

UPDATE triggers t
SET active = false
FROM trigger_rules r
WHERE r.trigger_id = t.id
  AND r.run_as_user_id IS NULL;
```

The migration does not backfill `run_as_user_id` from `created_by`.

It preserves active/paused state for manual Loops that already store an explicit run-as user.

#### Public response model

Extend `Loop` in `crates/api-schemas/src/loops.rs`.

| JSON field | Type | Contract |
|---|---|---|
| `configSource` | `"git" \| "manual"` | Configuration authority; distinct from existing `source`, which remains the Schedule/Subscription firing condition. |
| `syncedFromGit` | `boolean` | Convenience value derived from `configSource === "git"` and `deleted === false`. |
| `sourceRepositoryId` | `string \| null` | Retained Git provenance. |
| `sourceKey` | `string \| null` | Stable Loop directory key. |
| `sourceConfigPath` | `string \| null` | Retained path to `LOOP.md`. |
| `deleted` | `boolean` | Deleted override tombstone. |
| `executionState` | `"awaiting_approval" \| "active" \| "paused"` | Derived from `active` and `rule.runAsUserId`; never separately persisted. |

`source` remains the existing untagged `LoopSource` Schedule/Subscription payload for compatibility.

`configSource` is required because “source” cannot safely name both provenance and firing condition.

Update `CONTEXT.md` to use **Loop configuration source** for Git/manual authority and to reserve Schedule/Subscription for the firing condition.

#### Sync result model

Extend `LoopSyncSummary`.

| Field | Meaning |
|---|---|
| `created` | New Git Loops created inactive and awaiting approval. |
| `updated` | Existing Git-controlled Loops whose Git configuration was replaced. |
| `removed` | Existing Git-controlled Loops confirmed absent and deleted. |
| `approvalsRevoked` | Updated Loops that previously had a run-as user or were active. |
| `skipped` | Per-key reason records for invalid files, missing integration dependencies, live overrides, and tombstones. |
| `warnings` | Per-key ignored-authority warnings and other non-blocking interpretation warnings. |

A whole-fetch or database failure is an error response, not an empty `LoopSyncSummary`.

`SyncConfigResponse` retains separate `loops` and `workspaces` summaries.

### Interfaces and contracts

This section is included because administrators, ordinary users, the moat CLI, the frontend, Repository sync, stored clients, and firing workers depend on the behavior.

#### External HTTP contract

All mutation routes require `AuthUser.role == Some("admin")`.

Authorization runs before provider access and before persistence.

Reads retain current Scope filtering.

| Method and path | Request | Success | Rejection and invariant |
|---|---|---|---|
| `GET /api/v1/loops` | Existing filters; deleted rows omitted for non-admins and included for admins. | Scoped `Loop[]` with provenance and execution state. | Existing auth/Scope behavior. |
| `GET /api/v1/loops/{id}` | None. | Scoped `Loop`; admin may read tombstone. | Not found when Scope denies read. |
| `POST /api/v1/loops` | Existing create fields. Default `active` becomes false. `active = true` requires `rule.runAsUserId`. | Manual Loop. | Forbidden for non-admin; bad request for active without valid user. |
| `PUT /api/v1/loops/{id}` | Non-lifecycle config fields. | Updated manual/overridden Loop, preserving execution state. | Forbidden for non-admin; conflict for Git-synced or deleted row; lifecycle fields are not accepted here. |
| `DELETE /api/v1/loops/{id}` | None. | `204`. Plain manual Loop is deleted; Git-origin Loop becomes tombstone. | Forbidden for non-admin. |
| `POST /api/v1/loops/{id}/activate` | `{ "runAsUserId": "<user-id>" }` | Updated active Loop. | Forbidden for non-admin; bad request for unknown user; conflict for deleted row. |
| `POST /api/v1/loops/{id}/pause` | None. | Updated paused Loop. | Forbidden for non-admin; conflict when no approved user exists. |
| `POST /api/v1/loops/{id}/resume` | None. | Updated active Loop. | Forbidden for non-admin; conflict when no approved valid user exists. |
| `POST /api/v1/loops/{id}/override` | Optional non-lifecycle `UpdateLoopRequest`; empty body is allowed. | Manual Loop retaining provenance and execution approval. | Forbidden for non-admin; conflict for plain manual or deleted row. |
| `POST /api/v1/loops/{id}/reset-git` | None. | `ResetGitLoopResponse { loop, resynced, error }`; also restores a tombstone. | Forbidden for non-admin; bad request when no Git provenance; `resynced = false` preserves local state. |
| `POST /api/v1/repositories/{repository_id}/sync-config` | None. | Existing Repository plus extended Loop and Workspace summaries. | Forbidden for non-admin; provider/fetch/database errors are surfaced. |

The deprecated `/api/v1/triggers` aliases register the lifecycle routes against the same handlers.

Do not document the aliases in OpenAPI.

Remove `active` from `UpdateLoopRequest`.

Remove `run_as_user_id` from `UpdateLoopConfig`.

Lifecycle endpoints become the only update path for execution approval.

Keep both fields on `CreateLoopRequest`/`CreateLoopConfig` so an administrator may create and activate a manual Loop atomically.

Change create default from active to inactive.

#### moat CLI contract

The CLI is not a separate authorization boundary.

Backend role enforcement is authoritative.

| Command | Behavior |
|---|---|
| `moat loops list` | Show configuration source, execution state, and deleted status in human output; JSON uses the HTTP response. |
| `moat loops activate --id <id> --run-as-user-id <user>` | Atomically select user and activate. |
| `moat loops pause --id <id>` | Pause and retain user. |
| `moat loops resume --id <id>` | Resume the retained user. |
| `moat loops override --id <id> [config flags]` | Take local configuration authority, preserving provenance and execution approval. |
| `moat loops reset-git --id <id>` | Reset an override or restore a tombstone; print `resynced` and the error when unchanged. |
| `moat loops update --id <id> ...` | Edit only manual/overridden configuration; remove `--active`, `--run-as-user-id`, and `--clear-run-as-user-id`. |
| `moat loops delete --id <id> [--yes]` | Explain whether deletion is permanent or creates a restorable Git tombstone. |
| `moat repos sync-config <repo-id>` | Require admin through the backend; print created, updated, removed, approvals revoked, skips, warnings, and existing Workspace outcomes. |

#### Admin frontend contract

Use the existing Loop list/detail layout.

Use Workspace Git provenance UI as prior art, not as a shared component: the labels and execution consequences differ.

| Surface | Required behavior |
|---|---|
| Loop sidebar | Show configuration badge (`Git`, `Override`, `Manual`, `Deleted`) and execution badge (`Awaiting approval`, `Active`, `Paused`). Deleted tombstones remain visible to admins. |
| Loop detail header | Replace the raw active switch with lifecycle actions appropriate to the derived execution state. |
| Execution panel | Use `useAdminUsersList`; show login/name and Bot marker; activation requires one selected user; invalid stored user is visibly repairable. |
| Git source banner | Show Repository name and `sourceConfigPath`; Git state offers Override to edit; override offers Reset to Git; tombstone offers Restore from Git. |
| Loop edit form | Git-synced and deleted rows are read-only; manual/overridden rows are editable. Remove raw Run As User ID and active controls from the config form. |
| Delete dialog | Explain permanent deletion for plain manual Loops versus restorable tombstone for Git-origin Loops. |
| Repository sync | Confirm that all non-overridden Git Loop approvals will be revoked; state that Loop overrides/tombstones are skipped while current Workspace overrides are still reclaimed. |
| Sync result | Report created, updated, removed, approvals revoked, skips, warnings, and Workspace reclaim outcomes. |

Regenerate `packages/api-client` with `bun run generate:api`.

Frontend wrappers remain in `apps/frontend/src/features/loops/api/loops.api.ts`.

TanStack Query keys remain in `queryKeys.loops`.

Every lifecycle success invalidates Loop list, target detail, and recent Tasks only where the existing hook already does so.

Repository sync continues to invalidate Repositories, Workspaces, and Loops.

### Failure, safety and recovery

This section is included because Git/provider dependencies can fail, synchronization can partially classify input, writes can race, authorization can reject security-sensitive actions, and failure can create recurring Tasks under the wrong identity.

| Condition | State before | State after | Caller sees | Retry and recovery |
|---|---|---|---|---|
| Non-admin mutation | Any. | Unchanged. | `403 Forbidden` with stable access-denied meaning. | Retry only as an administrator. |
| Git fetch/auth/transport failure | Existing Repository Loops. | All Loops unchanged. | Repository sync fails; no empty success summary. | Safe to retry; provider error is logged with Repository ID and cause. |
| One malformed frontmatter file | Existing or new key is present. | Existing row and approval for that key unchanged; valid siblings may commit. | Skip naming key and parse reason. | Fix Git and retry. |
| Missing Adapter connection or invalid Schedule | Existing or new key is present. | Existing row and approval for that key unchanged; valid siblings may commit. | Skip naming missing dependency or invalid field. | Configure dependency/fix Git and retry. |
| Ignored `active`/`runAsUserId` | Valid Git file. | Synced inactive/unapproved. | Warning naming fields and Loop key. | No retry needed; remove misleading fields from Git. |
| Database failure during reconcile | Valid plan exists. | Entire Loop reconcile transaction rolls back, including tag creation, removals, and revocations. Workspace sync result follows its existing independent behavior. | Sync fails with causal error redacted at transport and logged once. | Safe to retry. |
| Repository sync meets override/tombstone | Local provenance row exists. | Row unchanged. | Skip reason identifies override or deleted override. | Use per-Loop reset/restore when intended. |
| Reset target absent/invalid in Git | Override/tombstone exists. | Unchanged. | `resynced = false` and actionable reason. | Fix Git or keep/delete local state. |
| Activate unknown user | Inactive Loop. | Unchanged. | Bad request naming invalid selection without exposing unrelated user data. | Select an existing user. |
| Stored user becomes invalid | Active/paused Loop. | Stored state remains for diagnosis; firing does not create a Task. | Admin UI shows invalid user; operational log identifies Loop and target ID. | Activate with a valid user. |
| Concurrent sync and activation | Git-origin Loop. | One complete ordered state. | Later committed transition wins according to the documented lock order. | Safe to retry desired final action. |
| Process dies during sync | Fetch or transaction in progress. | No transaction commit means no Loop changes; a committed transaction is complete. | Request fails or connection closes. | Safe to retry; no idempotency key required because reconciliation is key-based. |

The Repository Loop reconciliation transaction is idempotent by `(source_repository_id, source_key)`.

Activate, pause, resume, override, delete, reset, and restore are idempotent when repeated against their resulting state or return a stable conflict explaining the current state.

Do not automatically retry HTTP Git fetches inside the request.

The existing 30-second client timeout remains the upper bound unless Repository fetch infrastructure already supplies a shared policy.

Log one structured completion record with Repository ID and all summary counts.

Log skips and warnings at a level that does not produce an alert for expected override/tombstone behavior.

Log authorization rejection without request body contents.

Rollback is the forward migration that sets all Loops inactive before reverting code.

Do not roll schema back while rows use `config_source`, renamed `source_key`, or tombstones.

### Testing and verification

This section is always included.

The highest-leverage backend seam is the `LoopStore` reconciliation/lifecycle interface.

It exercises transactions, provenance, execution state, and concurrency without duplicating SQL assertions across HTTP callers.

HTTP integration tests cover authorization and the public contract.

Frontend tests attach to feature hooks/components.

E2E attaches at the CLI/API seam because the product guarantee is “Git alone cannot create a Task.”

#### Unit tests

| Module | Cases |
|---|---|
| `backend/src/loops/sync/parser.rs` | Valid Git authority fields produce warnings; malformed files retain directory identity; `active` and run-as never enter desired execution state. |
| `backend/src/loops/sync/mod.rs` | Classification distinguishes valid, skipped-present, and confirmed-absent keys; Repository branch is passed to fetch; no automatic fallback user resolution remains. |
| `crates/api-schemas/src/loops.rs` | JSON names for `configSource`, provenance, `executionState`, lifecycle requests, warnings, and summaries. |
| `crates/cli/src/commands/loops.rs` | Lifecycle payload construction, removed generic flags, human status output, reset result, and delete wording. |
| Frontend Loop utilities | Derived provenance/execution labels and read-only rules. |

#### Backend database integration tests

Extend `backend/tests/loops_sync_test.rs`.

Add focused handler/lifecycle tests if that file becomes too broad.

| Case | Assertion |
|---|---|
| New Git Loop import | Git config persists; active false; run-as null; provenance complete. |
| Git authority fields | Values ignored; warnings emitted. |
| Re-sync active Loop | Config updates; active false; run-as null; `approvalsRevoked` increments even for unchanged Git content. |
| Override then Repository sync | Override config and execution state unchanged; skip reported. |
| Reset override | Only target key returns to Git; sibling untouched; approval cleared. |
| Delete Git Loop | Tombstone persists, provenance retained, approval cleared. |
| Repository sync with tombstone | Tombstone unchanged; skip reported. |
| Restore tombstone | Target returns to Git inactive/unapproved. |
| Malformed existing key | Existing config and approval preserved; valid sibling syncs. |
| Missing connection existing key | Existing config and approval preserved; valid sibling syncs. |
| Confirmed absent Git key | Git-controlled row deleted; override/tombstone not deleted. |
| Injected write failure | No valid sibling, removal, tag, or approval change commits. |
| Concurrent activate/re-sync | State equals one serial commit order; no active Loop without user. |
| Migration fixture | Git rows all inactive/unapproved; manual null-user rows inactive; manual explicit-user rows preserve active/paused state. |

#### HTTP contract and authorization tests

| Actor and action | Expected |
|---|---|
| Non-admin list/get accessible Loop | Success according to Scope. |
| Non-admin create/update/delete/lifecycle/sync | Forbidden with no state change. |
| Admin create inactive manual Loop | Success. |
| Admin create active without user | Bad request. |
| Admin activate with valid human or Bot user | Active with explicit ID. |
| Admin pause/resume | User retained; active toggles. |
| Admin update Git-synced Loop | Conflict directing override. |
| Admin reset absent/invalid key | Success response with `resynced = false`; state unchanged. |
| Deprecated Trigger alias lifecycle call | Same response and authorization as Loop path. |

#### Firing tests

| Path | Cases |
|---|---|
| Schedule claim and execution | Active+user fires as that user; inactive, null-user, deleted, and invalid-user Loops do not create a Task. |
| Subscription matching and inbound creation | Active+user match fires as that user; inactive, null-user, deleted, and invalid-user Loops do not match/create. |
| Ongoing routing | Existing `ongoing_task_id` is preserved across Git config replacement only as runtime state, but re-sync still clears approval before another delivery. |

#### Frontend tests

Add unit tests beside the affected feature components.

| Surface | Cases |
|---|---|
| `AdminLoopsSidebar` | Git/manual/override/deleted badges; awaiting/active/paused badges; deleted visible to admin. |
| Git source banner | Override, reset, and restore calls; provenance label; loading and error states. |
| Execution panel | Named user/Bot selection; activate, pause, resume; invalid stored user; pending state. |
| Loop form | Git/deleted read-only; manual/override editable; no raw run-as or active controls. |
| Repository sync UI | Destructive approval-revocation confirmation; Loop summary and existing Workspace reclaim warning. |
| Hooks/API wrapper | Correct generated endpoint, invalidation, and error propagation for every lifecycle action. |

#### Product scenarios and E2E

Add `specs/loops/00-design.md`.

Add `specs/loops/01-loop-configuration-scenarios.md` with permanent `LOOP-*` IDs.

Map the scenarios to `tests/api/specs/loop-git-configuration.spec.ts`.

Minimum acceptance scenarios:

| ID placeholder | Given | When | Then |
|---|---|---|---|
| `LOOP-01` | Git declares a Schedule Loop with `active: true` and a user. | Admin syncs. | Loop is awaiting approval and no scheduled Task appears. |
| `LOOP-01.1` | `LOOP-01` completed. | Admin activates with a selected user. | The next eligible firing creates a Task as that user. |
| `LOOP-01.2` | The Loop is active. | Admin re-syncs unchanged Git. | Approval is revoked and later firing creates no Task. |
| `LOOP-02` | Git Loop is active. | Admin overrides and edits locally. | Approval remains; Repository sync skips local configuration. |
| `LOOP-02.1` | `LOOP-02` completed. | Admin resets to Git. | Local edit is replaced and approval is revoked. |
| `LOOP-03` | Git Loop exists. | Admin deletes it and later syncs the Repository. | Tombstone remains and the Loop is not recreated. |
| `LOOP-03.1` | `LOOP-03` completed and Git still declares the key. | Admin restores from Git. | Loop returns awaiting approval. |
| `LOOP-04` | One valid and one malformed Loop exist, with a prior row for malformed key. | Admin syncs. | Valid key syncs; malformed key and its approval remain unchanged; skip is reported. |
| `LOOP-05` | Ordinary user can read a user-scoped Loop. | User tries every mutation class. | Each is forbidden and the Loop remains unchanged. |

#### Manual verification

A person must judge the following in the admin UI:

- Configuration-source badges are distinguishable from execution-state badges.
- Awaiting approval cannot be mistaken for merely paused.
- The user picker shows names/logins and Bot identity without requiring copied UUIDs.
- Repository sync confirmation makes approval revocation clear before the action.
- Override, reset, delete, and restore copy explains which configuration is replaced and whether approval survives.
- Skips and warnings identify the affected Loop without requiring logs.

#### Commands and tiers

Run the Rust schema/CLI/backend unit tier:

```bash
cargo test -p moatless-api-schemas
cargo test -p moatless-cli
cargo test -p moatless-vibe-backend
cargo clippy -p moatless-api-schemas -p moatless-cli -p moatless-vibe-backend -- -D warnings
```

Run the database integration tier.

This is the tier that actually builds `backend/tests/loops_sync_test.rs`.

```bash
cargo test -p moatless-vibe-backend --features db-tests --test loops_sync_test
```

Run generated-client and frontend verification:

```bash
bun run generate:api
bun run type:check
bun run biome:check
bun run test:unit
```

Run the Loop E2E chain against Docker when Docker is available:

```bash
task test:compose:e2e -- --suite api
```

Run the focused chain against an already-running profile:

```bash
E2E_PROFILE=docker bun run test:e2e -- loop-git-configuration
```

If Docker is unavailable in the task Sandbox, run direct Rust/frontend checks and report the omitted Docker/E2E tier.

## Decisions

| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| [1](#1-authority-from-git) | Authority from Git | Git supplies configuration only; imported Loops are not runnable. | Committed `active` and `runAsUserId` would let Repository code grant recurring execution authority. |
| [2](#2-activation-action) | Activation action | Administrator selects run-as user through UI or moat CLI. | Ordinary-user or Git activation violates the approval boundary. |
| [3](#3-re-sync-approval) | Re-sync approval | Every administrator Git sync revokes approval, including unchanged sync. | Preserving approval across no-op sync makes approval depend on content comparison and weakens the explicit re-approval rule. |
| [4](#4-local-override) | Local override | Repository sync skips overrides; only per-Loop reset returns to Git. | Repository-wide reclaim can replace unrelated local Loop edits; a sync-time prompt diverges between UI and non-interactive CLI. |
| [5](#5-automatic-reconciliation) | Automatic reconciliation | Remove Loop sync from Repository creation and Task start. | Automatic no-op or material sync conflicts with administrator-only synchronization and can revoke approval during unrelated Task work. |
| [6](#6-loop-mutation-authority) | Loop mutation authority | Every Loop mutation is administrator-only; reads still follow Scope. | User-owned mutation and Git-only restriction leave alternative mutation paths outside centralized control. |
| [7](#7-pause-versus-revoke) | Pause versus revoke | Pause retains the run-as user; Git sync/reset and delete revoke. | Clearing on every pause makes temporary operational suspension require identity approval again; offering both choices adds unnecessary state transitions. |
| [8](#8-delete-a-git-origin-loop) | Delete Git-origin Loop | Keep a deleted override tombstone and restore explicitly. | Hard delete allows later recreation; requiring override before delete adds a meaningless preparatory action. |
| [9](#9-override-and-approval) | Override and approval | Override preserves active/paused state and run-as user. | Override changes configuration authority under admin control; revoking execution would conflate independent state axes. |
| [10](#10-sync-failure-policy) | Sync failure policy | Sync valid siblings, preserve skipped present Loops, and atomically commit accepted changes. | Whole-sync logical failure reduces useful recovery; per-Loop commits permit partial repository state. |
| [11](#11-explicit-execution-identity) | Explicit execution identity | Every active Loop stores a run-as user; migration deactivates every Loop without one. | Creator fallback remains implicit authority; backfilling it preserves execution without new administrator approval. |
| [12](#12-authority-fields-in-git) | Authority fields in Git | Ignore `active` and `runAsUserId`, sync inactive/unapproved, and return warnings. | Trusting them violates Decision 1; silent ignore leaves Repository authors believing the fields work. |

### 1. Authority from Git

The trust boundary is the core requirement.

`LOOP.md` may define firing condition, routing mode, prompt, tags, agent type, inactivity TTL, Adapter matching/config, and Schedule configuration.

It may not define whether the Loop is runnable or whose identity it uses.

The cost is that every new Git Loop requires a second administrator action after sync.

That cost is intentional.

### 2. Activation action

Selecting the user and setting `active = true` is one atomic action.

The UI uses a named user picker.

The CLI requires `--run-as-user-id`.

The HTTP interface validates the target before writing either field.

The rejected alternative was keeping the current independent generic fields.

That interface permits `active = true` to be written without proving an explicit identity.

### 3. Re-sync approval

Every accepted Git replacement revokes approval.

No content hash or equality optimization preserves execution state.

This makes the rule observable and auditable: an administrator who asks to sync must approve again.

The cost is operational interruption after an unchanged explicit sync.

Repository sync confirmation and result counts must make that cost visible.

### 4. Local override

An override is durable local configuration authority with retained Git provenance.

Repository-wide Loop sync respects it.

Reset operates on one Loop.

This differs from the current Workspace explicit-sync rule, where Repository sync can reclaim live Workspace overrides.

The difference is deliberate and must be explicit in Repository sync copy and CLI output.

The cost is that returning many Loop overrides to Git requires multiple reset actions.

### 5. Automatic reconciliation

Repository creation and Task start stop invoking Loop reconciliation.

Task start continues automatic Workspace reconciliation.

Explicit Repository sync remains the import/re-sync entry point.

The cost is that newly committed Loops do not appear until an administrator syncs.

This is required to keep synchronization itself under administrator control.

### 6. Loop mutation authority

Create, update, delete, activate, pause, resume, override, reset/restore, and Repository config sync all require administrator role.

Scope remains a read boundary.

Existing user-scoped Loops remain visible to their owner but become administrator-managed.

The cost is a compatibility change for users who currently create or manage personal Loops through the API/CLI.

There is no grandfathered mutation right.

### 7. Pause versus revoke

Paused means approved but not firing.

It retains `run_as_user_id`.

Resume may therefore set `active = true` without another user selection.

Git import/re-sync/reset/restore and deletion are authority-changing operations and clear approval.

The cost is that `active = false` alone no longer tells the whole story; clients must use the derived execution state.

### 8. Delete a Git-origin Loop

Git-origin deletion is a tombstone, not a cascade.

It preserves the stable key and provenance so later Repository sync skips it.

Restore is the same per-Loop Git read used by reset.

The cost is retained database rows and an admin-only deleted listing.

Plain manual Loops continue hard deletion.

### 9. Override and approval

Override is performed by an administrator and changes only which configuration is authoritative.

It preserves active/paused state and the selected user.

Reset changes configuration back to Git and clears approval.

The rejected alternative treated any source transition as security-sensitive.

That would make harmless local editability changes unexpectedly stop work.

### 10. Sync failure policy

Logical errors are isolated per present key.

They are not deletion evidence.

Valid siblings may still reconcile.

All accepted writes and confirmed removals share one transaction.

A database error rolls them all back.

The cost is a more structured fetch/parse plan and transaction-aware tag resolution.

That complexity concentrates in the Loop sync/store seam instead of every caller.

### 11. Explicit execution identity

There is no creator fallback after this change.

An active Loop with no stored user is invalid and cannot fire.

Migration deactivates such rows instead of converting `created_by` into approval.

The cost is immediate suspension of existing implicit-identity Loops until an administrator reviews them.

That is the desired rollout safety property.

### 12. Authority fields in Git

`active` and `runAsUserId` remain recognizable during parsing only to produce warnings.

They never enter the desired Git Loop representation.

The Loop still syncs inactive and unapproved.

Rejecting the entire file would preserve a previously active row and turn a harmless stale field into a denial of unrelated config updates.

Silently ignoring the fields would make Git authoring behavior misleading.

## Out of scope

- Changing Schedule cron semantics, Subscription matching semantics, or Routing mode.
- Changing current Workspace override, tombstone, reset, or Repository-wide reclaim behavior.
- Extending Loop Git fetch beyond its current GitHub support.
- Redesigning Adapter connections or Task ownership.
- Removing `/api/v1/triggers`.
- Moving legacy `triggers`, `trigger_rules`, `adapter_event_trigger_sources`, or `schedule_trigger_sources` tables into newly named Loop tables.
- Changing `ongoing_task_id` runtime semantics.
- Adding audit-event storage beyond existing timestamps, logs, and sync results.
- Producing implementation tickets in this specification session.

## Open questions

- None.

## Ticket breakdown

None.
