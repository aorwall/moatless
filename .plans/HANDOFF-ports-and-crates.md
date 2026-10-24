# Handoff — backend ports and crates

> **For:** an agent starting this work with no prior conversation.
> **Spec:** `.plans/backend-ports-and-crates.md` is authoritative for *what* to build. This file is
> operational: where the work stands, how the lanes divide, and the rules that were learned the hard
> way and are not derivable from the code.
> **Updated:** 2026-08-09 · main at `299fca86dff9`, census ceiling **0**. The carve frontier is complete.

## What this program is

Two halves, in order:

1. **Make the ports singular** — remove every illegal reference where a lower-band module reaches up
   into a higher band. Measured by `scripts/crate-census-baseline.txt`, a ratchet. Started at **83**.
   **Now 0. This half is done.**
2. **Split the backend** — extract each band into its own crate. All thirteen carve-target crates
   are extracted, including the backend/server crate. **This half is done.**

The census only counts *production* reference edges (`CODE`/`ATTR`/`MACRO`). Doc paths and
`#[cfg(test)]` edges are excluded by design — see "test-context" below.

## Where it stands

**Main:** `299fca86dff9`. **Ceiling: 0** — read from the baseline blob on `main`, not from a report.
There are no illegal production edges left in the repository.

A SHA here is a write-time snapshot and cannot include the change that records it, so treat it as
"the state this file describes" rather than as today's `main`. What must stay true is that the two
places naming it agree; the ceiling and the gates are what you actually verify against.

**Doc-path count is also 0.** The two `[DOC]` intra-doc paths this section used to list
(`github_pr/adapter.rs:107`, `telegram/adapter.rs:80` → `crate::compose`) are gone. They were never
Cargo edges and were never counted; both cleared as their carves went through.

**FRONTIER is complete.** The report says
`FRONTIER -> complete: every carve-target crate is extracted.` All thirteen carve targets are
extracted: identity, workspaces, tasks, sandbox-control, sandbox-k8s, sandbox-docker, loops, the five
Adapters, and server. The backend carve landed in #487; `server` is now an extracted crate.

**All three lanes are closed.** History, because the PR numbers are the only durable record of why
each shape is what it is:

| Lane | Outcome |
|---|---|
| **linear** | **Done.** #456 (L1 webhook) → #461 (L2 api_routes) → #464 (reshape) → #465 (L3 issues) → #467 (L4 delivery+sessions). Zero `adapter-linear` edges. |
| **github_pr** | **Done.** #457 (handlers ingress), #458 (Connector + dead registry teardown), #460 (api_routes), #463 (shell_reconcile), then #469 — the four-edge router slice that took 4 → 0. |
| **loops carve** | **Done.** #466, the first crate this program split out. Landed illegal-neutral. |
| **backend carve** | **Done.** #487, the last carve. Landed with FRONTIER complete, ceiling 0, doc paths 0. |

### Crates carved so far

Each landed illegal-neutral against a ceiling of 0. The PR is the durable record of the shape.

| Crate | PR | What it added to the recipe |
|---|---|---|
| `moatless-loops` | #466 | The first split. Ambient context does not travel — five red runs on things the graph does not model. |
| `moatless-adapter-webhook` | #476 | The provider recipe: facade at `providers/<name>.rs`, visibility priced by incoming callers, `providers/mod.rs` untouched. |
| `moatless-adapter-slack` | #478 | Two `db-tests`-gated modules of its own, which is what exposed the CI gap fixed in #479. |
| `moatless-adapter-telegram` | #480 | `db-tests` as a pure feature-forward with no gated modules. The census self-test became a synthetic fixture; the real-tree guard stayed real. |
| `moatless-adapter-linear` | #484 | The external spelling. Five modules had to be re-exported on the strength of `backend/tests/integration/` alone, which an in-crate `crate::` sweep never sees. |
| `moatless-adapter-github-pr` | #483 | The dead-manifest-dependency check, corrected twice — `src/` is not the crate, and a shared dependency is invisible to every carve but the last. |
| `moatless-identity` | #387 | The base case: first real band crate, following #385's census precursor. |
| `moatless-workspaces` | #390 | The first big visibility-price point; incoming edges stayed census-invariant while outgoing edges collapsed. |
| `moatless-tasks` | #429 | The carve that exposed why the `floor_crate_of_dir` and `external_crate` arms must land with the move. |
| `moatless-sandbox-control` | #435 | The control band separated from the runtime providers with the provider SDKs kept out of the control error surface. |
| `moatless-sandbox-k8s` | #436 | Kubernetes runtime provider extracted together with the Docker provider. |
| `moatless-sandbox-docker` | #436 | Docker runtime provider extracted and the sibling-provider edge cleared. |
| `moatless-backend` | #487 | The final server/backend carve; `main` at `299fca86dff9` reports FRONTIER complete. |

**All thirteen carve targets are extracted.** `backend/src/adapters/providers/` is now five facade
files and `mod.rs`, with no provider directory left under it; `server` is also carved.

## What to do next

1. ~~Backfill `external_crate("moatless_loops")`.~~ **Done — #471.** Kept in the list because the
   *reason* it mattered generalises to every future carve; see Known gaps.
2. ~~The five Adapter carves.~~ **Done — #476, #478, #480, #484, #483.**
3. ~~The backend/server carve.~~ **Done — #487.**
4. **Close the cleanup PRs.** The remaining work is documentation/tooling cleanup, not carve work:
   make dead census defenses explicit or delete them, record the refused `moatless-sandbox` rename,
   and report whether the planned `tools/crate-census` deletion would lose the band-order ratchet.

The ratchet is at 0, so any carve that introduces an illegal edge fails immediately — which is the
point. The thing that does *not* fail is an edge the census never scored at all.

## The slice that closed the ports half — github_pr four-edge router (#469)

**Merged.** Kept here because it is the worked example of every rule below, and because the next
provider carve will face the same shape. The design as built:

**Shape:** a `GithubPrInboundRouter`, constructed once at `compose`, holding ~13 downward ports as
private fields, with the four delivery handlers as **methods** (`route_event` + `close_subject` in
`routing.rs`; `run_merge_conflict` / `run_post_merge` as impl blocks in their own files so the heavy
logic stays put). `delivery::register` captures `Arc<router>` and the closures call the methods.
`merge_conflict`/`post_merge` take `&self` plus the adapter for auth. One
`#[expect(clippy::too_many_arguments)]` on `new()` — the width lives at construction, not per call.

**It shipped as two commits, in this order** — the split is the point, and a future slice of this
shape should copy it:

1. **Band-5, standalone-readable:** `TaskCloser` gains a close entry taking
   `CloseEvent { actor: Option<String>, payload: serde_json::Value }`, forwarded to
   `publish_task_event`. `close_for`/`close_tree` pass `CloseEvent::neutral()` (`actor=None` →
   `row.agent_type`, `payload={}`) so every existing caller is byte-unchanged. The payload stays
   **opaque to `tasks`** — the provider builds `{reason, pr_number}` itself. Deliberately *not* a
   typed `CloseCause` enum: that would drag provider specifics down into band 5.
2. **Band-9 slice:** the router. `close_subject` delegates termination to the new entry with
   `actor: Some("claude-code")` and `{reason:"pr_closed", pr_number}`.

**Three findings that shaped it, each verified by reading — do not re-derive:**

- **The registry read dissolves.** `route_event` reached `state.adapters` only to fetch github_pr's
  *own* `reply_instructions`, which is a pure constant (`adapter.rs:244-265`, `&self` unused). Resolve
  it from the provider's own presentation; the router never holds `Arc<Adapters>`.
- **`close_subject` is a partial duplicate.** Its *termination core* duplicates
  `TaskCloser::close_single` step for step (verified including the `deactivate` helper body). Its
  *wrapper* is genuinely github_pr's — binding-origin/`loop_id` selection, the sibling guard, and a
  close-notice append that **must** precede the status flip because `send.rs` drops inbound on
  `Closed`. Wrapper stays; core delegates. So the six `TaskCloser` ports never reach the router.
- **The actor must be an override, not a fix.** `agent_type` comes from the loop config
  (`inbound.rs:101`), which is user-configurable via the HTTP handler and sync frontmatter where
  `"claude-code"` is a **default, not a constraint**. Switching to `row.agent_type` would change a
  persisted, wholesale-forwarded observable. The "who really closed this task" question (the closer
  is a webhook, not an agent) is filed separately and must not ride in on this refactor.

**Why the payload could not be dropped:** `reason` is *rendered*, not telemetry —
`to-lifecycle-event.ts:51` reads it, `getDisplayMessage` falls back to it, `event-log.tsx:47` draws
it. A github_pr close shows "Task closed pr_closed" today. `pr_number` is dead on the read side but
still counts, because it rides wholesale through four sinks including durable Postgres persistence.

## The loops carve — PR #466, merged

The first crate this program split out. It landed **illegal-neutral**: the census baseline blob is
byte-identical to `main`, UNRESOLVED is 0, and `loops` scores as its own band-8 node.

Two things about that neutrality claim are worth copying, because both were nearly got wrong:

- **It was re-measured, not carried.** The branch was first verified neutral against base
  `1d0138e` (ceiling 6, shared blob `1db9ae2`); by merge time the base was `14d4f0d6b` (ceiling 4)
  and the shared blob was `8c8529ac`. The blob **value** moves with the base; the
  identity-to-`main` **property** is what neutrality means. A delta measured against a stale base is
  a stale delta — see "The census number is base-relative" below.
- **Neutrality means the baseline regenerates to `main`'s blob on its own** — not that it was edited
  until it matched. Those two produce the same file and mean opposite things.

The extraction was sound from the first push, but **CI was red anyway**, on five mechanical failures.
They are listed here in full because none of them is about the graph the census models, and the next
carve will hit the same class:

1. **Plan claims** — six assertions pinned to moved paths (`:198`, `:199`, `:559`, `:563`, `:564`,
   `:565`). Repoint at `crates/loops/src/...`. Do not delete them.
2. **`bindings-adapters-baseline.txt`** — five stale entries under `backend/src/bindings/`. **Repoint,
   do not delete.** There is a real question underneath (bindings and adapters are now the same
   crate, so the untangle invariant became intra-crate) but it must not be answered inside the PR
   that trips it. It is still open — filed, not answered.
   **The trap:** repointing the baseline entries alone does not fix it. The checking script computes
   its "now" set from `find $DIR`, and `DIR` pointed at the vanished `backend/src/bindings`. A
   baseline and the script that reads it are **one artifact**; move both or the gate silently
   measures an empty directory.
3. **`tools/crate-census/src/main.rs:1450`** — `carved_but_unbanded_fires_...` expects `floor:loops`,
   gets `loops`. That test simulates a *missing* bands arm and was keyed on `loops`; adding the arm
   stopped the simulation simulating. Repoint it at a still-unbanded directory.
4. **`crates/loops/src/loops/sync/parser.rs:226`** —
   `Path::new(env!("CARGO_MANIFEST_DIR")).join("../.moatless/loops")`. From `backend/` that reached
   the repo root; from `crates/loops/` it reaches `crates/`. Needed one more `../`. **This is the one
   that no build could have caught** — see the fourth carrier below.
5. **`backend/Cargo.toml`** — the `db-tests` feature had to forward `moatless-loops/test-support`.
   Found late, because `--features db-tests` on the *backend* masked that the library-side feature no
   longer followed the moved code.

### Ambient context does not travel

The census models the graph *between* files — who references whom, in which direction — and
extraction does not change that graph (proven: every legal-down edge resolved exactly). What
extraction changes is the **ambient context** a file sits in, and everything ambient is structurally
invisible to a reference census. Four carriers so far:

| Carrier | Fails at |
|---|---|
| **Visibility** — `pub(crate)` means a different caller set across the seam (62 widenings, 111 `E0603`/`E0624`) | compile |
| **Crate-root attributes** — the `#![cfg_attr(test, allow(...))]` lint allow does not move with the file | clippy only |
| **Feature context** — `test-support` / `db-tests` dev-dep gates | compile |
| **Filesystem paths relative to the package root** — `CARGO_MANIFEST_DIR` offsets | **run time** |

The fourth is why the checklist item is *"does `--all-targets` build **and do the tests run**"*, not
just "does it build".

### Carve checklist

- **Pre-flight:** grep the band's test files for constructions of types *above* the band. A
  `#[cfg(test)]` edge is invisible to the census by design and only surfaces at extraction.
- Enumerate **every** extern crate name referenced and subtract the manifest. Do not curate a suspect
  list — that is how `urlencoding` and `serde_yaml` were missed.
- Add **both** `bands.rs` arms **in the same slice as the `git mv`**. A real-band carve needs two,
  and they fail in opposite directions:
  - `floor_crate_of_dir("<name>") => "<name>"`. Without it the new dir scores as `floor:<name>` at
    band 0, which flips every legal downward edge to illegal *and* launders future upward edges.
    The `main.rs` carved-but-unbanded backstop catches this one's absence, loudly.
  - `external_crate("moatless_<name>") => "<name>"`. **Nothing catches this one's absence.** With it
    missing, every `moatless_<name>::` reference from another crate resolves to `None` and is
    **unscored**: present edges vanish from the reported dependency lists, and a future illegal edge
    into the carved crate is not reported as tolerated but not reported at all. `loops` shipped
    without it and it was backfilled at #471; `tasks` shipped without both at 10d, restored at 10e.
    **Add a `band_of` unit test pinning the head** in the same slice — without one, deleting the arm
    later regenerates a byte-identical baseline and passes.
  A carve that omits the second arm still measures ceiling-neutral on the day, because the only
  crates naming it are the ones that just moved. That is why it is easy to skip, why skipping it is
  invisible, and why the ceiling is the wrong place to look for it — check the **report** diff.
- **Sweep the carved name in both spellings, and price the facade off the external one.** An
  integration test under `backend/tests/` reaches the provider from outside the crate, so it spells
  the path `moatless_backend::adapters::providers::<name>` where in-crate code spells it
  `crate::adapters::providers::<name>`. A sweep keyed on the `crate::` prefix skips the external
  spelling silently, and the omission compiles in-crate — it fails only in the integration-test
  target. `linear` is the crate this bites hardest: `backend/tests/integration/` names
  `linear::adapter`, `linear::delivery`, `linear::delivery::register`,
  `linear::handlers::linear_webhook` and `linear::ingress`, so five modules must be re-exported by
  the facade and `pub` in the new crate on the strength of tests alone. Grep both spellings across
  `backend/`, `crates/` and `tests/`.
- Test-context edges are **not** ratcheted. Pricing them would fire on every legal in-crate test and
  create pressure to weaken tests to satisfy a counter.

## Known gaps — measured, and deliberately not fixed in the PR that found them

Each was found with a measurement, not a suspicion, and each was kept out of the PR that surfaced it.
That separation is deliberate: a fix folded into the diff that tripped it is a fix nobody reviews.
The first entry is now closed; it stays because how it was wrong generalises.

**`external_crate("moatless_loops")` was missing from `bands.rs` — closed by #471, and the way it
was wrong is the part worth keeping.** The loops carve landed `floor_crate_of_dir("loops")` but not
its pair, satisfying one of the carve rule's two arms.

It was first reported as *measured inert*, on good evidence: adding the arm and regenerating produced
a **zero-diff baseline**, ceiling unchanged, byte-identical. That evidence was real and the
conclusion drawn from it was too narrow. The baseline was unchanged; the **report** was not. Ten
lines moved, and they were a correction:

```
adapter-github-pr  deps: identity, tasks, workspaces        → identity, loops, tasks, workspaces
adapter-linear     deps: identity, sandbox-control, tasks   → identity, loops, sandbox-control, tasks
adapter-slack      deps: identity, tasks                    → identity, loops, tasks
adapter-telegram   deps: identity                           → identity, loops
adapter-webhook    deps: floor / adapter-model only         → loops
```

**The census was reporting five crates as not depending on `loops` when they do.** `adapter-webhook`
printed as having no band dependencies at all. A missing `external_crate` arm does not merely fail to
catch a *future* edge — it silently subtracts *present* ones from the reported graph.

That matters because `READY`/`WAITING` is derived from exactly those dependency lists, and the
frontier line is what tells you which crate to carve next. All five read `READY` both before and
after, and correctly so — but only because `loops` was already `EXTRACTED`. Had the arm been missing
while `loops` was un-extracted, the census would have printed five adapter crates as ready to carve
while each depended on an unextracted crate. **The bug class is "the tool tells you it is safe to
carve X", not "the tool fails to complain later."**

Two rules came out of it, both general:

- **Fix the instrument before the thing it measures moves.** Backfilled as its own one-line PR, the
  way 10e restored `tasks` — never inside a carve, or that carve changes the instrument and the
  subject in one step and there is no clean before.
- **An inert change still needs a test.** #471 added `moatless_loops_head_resolves_to_the_band_8_crate`
  (asserts the head resolves and `band_of == 8`) because without one, *removing* the arm again
  regenerates a byte-identical baseline and passes. The silent-subtraction failure the arm closes was
  itself unguarded. A change justified as "the ratchet does not move" is exactly the change the
  ratchet cannot protect.

**The bindings/adapters untangle invariant became intra-crate.** #466 put `bindings` and `adapters`
in the same crate, so the baseline that tracks their separation now guards a within-crate boundary
rather than a cross-crate one. Whether that gate still means what it meant is open. Filed by the
carve, deliberately not answered inside it. 5 files remain pending.

**Two `[DOC]` intra-doc paths point up** (`github_pr/adapter.rs:107`, `telegram/adapter.rs:80` →
`crate::compose`). Not Cargo edges, not counted, cleaned as Step 8f.

## What the lanes knew that the PRs do not say

Collected from the three lanes at handoff. Each of these is obvious while building and invisible
afterwards.

**The census number is base-relative.** A slice measured on a stale base carries a stale delta. The
github_pr router slice read `6 → 2` on the base it was designed against and `4 → 0` on the base it
merged on, because by then the edges it clears were the only ones left. On any rebase, let
`crate-census` write the number and never carry the old one forward. Two lanes hit this independently
within an hour, and a third — the loops carve — hit its blob-identity equivalent the same day.

**A baseline and the script that reads it are one artifact.** Repointing the five entries in
`bindings-adapters-baseline.txt` alone *fails*: the check computes its "named now" set from
`find $DIR`, and `DIR` still pointed at `backend/src/bindings`, which no longer exists — so an empty
set makes every repointed entry report as no-longer-naming `crate::adapters`. The script itself had
to move. The failure mode looks like the invariant being satisfied rather than the check being broken.

**`--features db-tests` can mask a missing library-side feature forward.**
`clippy --all-targets --features db-tests` compiles test targets where the dev-dependency is active,
which hides a facade re-export that is *library* code under that feature.
`cargo check --features db-tests --lib` is the unmasked path. This produced an earlier "green" that
was not.

**A file-wide `sed` is not a refactor.** `s/state\./self./` corrupts `pr.state.` into `pr.self.` and
`external_mergeable_state` into `external_mergeable_self`, and silently skips every multi-line
`state\n.field`. Any such rewrite needs a follow-up `grep -nE '\.self\.|_self\b'` audit and a hand
pass for wrapped receivers. This is read-every-occurrence in its most concrete form.

**The routers have two construction sites each, and only one announces itself.**
`LinearInboundRouter` is spelled field-for-field in both `compose.rs` and the
`drain_linear_adapter_delivery` test helper (`tests/integration/common/mod.rs`). This is deliberate —
there is no `from_appstate` constructor, precisely so `adapter-linear` names no `AppState` — but it
means any change to the field list must edit both, and **only `--all-targets --features db-tests`
catches the test one.** The census cannot see it.

**The routers' ports are tomorrow's edges.** They name ports spanning bands 5–8, all intra-server-band
and *uncounted* today. When tasks/bindings/loops carve, each becomes a real
`adapter-* -> {tasks,bindings,loops,sandbox-control}` edge and the adapter's `Cargo.toml` gains those
dependencies. They are believed all downward — the five adapters extract above those bands, server
last — but **verify against the census as each band lands rather than assume.** The router is each
provider's single widest dependency surface and the thing most likely to surprise on a carve.

**The two routers are aligned in shape, not method-for-method, and that is correct.** Both are
services with private ports, built at compose, `Arc`-shared, captured per delivery closure. But linear
registers five durable-delivery kinds so it has five kind-named methods, where github_pr has
`route_event` + `close_subject`. **Do not force either into the other's method count.** In particular
linear's close path is *not* a router method: `close.rs::close_active_bindings_for_issue` sits one
level below, reached from both `issues.rs` and `notifications.rs`, and two independent entry points
is why it is its own module. If the close-core delegation is ever picked up, linear's delegation
point is `issues.rs`/`close.rs`, not a router method. The providers legitimately place "close" at
different depths.

**Do not "simplify" the four registry pass-throughs.** `reactions`, `reactions_by_name`,
`outbound_for_binding_reply_by_name` and `outbound_for_message_reply_by_name` stayed `pub(crate)` and
are reached from the moved backend tests through thin delegating pass-throughs in
`test_support::registry`. Widening them to `pub` would silently undo the whole reason that option was
chosen: no accessor is public for a test's sake alone.

## Working protocol

**Design-then-report.** For any slice with a seam or a behaviour question: map it, report the design,
get a ruling, *then* build. Several rulings this program reversed a lane's lean; all of them were
cheaper before the branch than after.

**The five gates.** A subset gate reporting "clean" is the single most repeated failure here. Run all
five before every push:

```
./scripts/check-plan-assertions.sh
./scripts/check-crate-census.sh
./scripts/check-appstate-composition-root.sh
./scripts/check-bindings-adapters-baseline.sh
./scripts/check-dao-composition-root.sh
```

Plus `cargo clippy --all-targets --features db-tests -- -D warnings` and the test suite.

**Census discipline.**
- Never write the number. Run `cargo run -p crate-census` and let it write.
- On a baseline conflict: take *either* side, then **regenerate**. A hand-picked side is a number we
  authored sitting in a file whose entire authority comes from not being authored.
- After regenerating, `git status` and confirm the baseline file is **in the commit diff**. A
  measurement you ran and did not commit is indistinguishable, downstream, from one you did not run.
- **Check merge state before CI.** `DIRTY` and `CI in_progress` look identical if you only watch CI,
  and the merge base is the one that invalidates your measurement. This cost three rebases.

**PRs.** Squash-merge only. Create with `moat gh pr create` — **never `moat gh stack submit`**, which
bypasses the backend proxy so review comments never route back. `-R` and `--head` are inferred.

**gh token** goes stale; source it per call:
```bash
export GH_TOKEN=$(printf 'protocol=https\nhost=github.com\n\n' | git credential fill 2>/dev/null | sed -n 's/^password=//p')
```
`check-runs`/`statusCheckRollup` 403 for these tokens — use
`gh run list --branch <branch> --json headSha,workflowName,status,conclusion`. Superseded
pre-force-push runs show as `cancelled` alongside the real one, so key on `headSha`.

Prefer `--branch` over `--commit`. **`--commit` requires the full 40-character SHA; given an
abbreviated one it returns zero rows, exit status 0, no error** — indistinguishable from a commit
with no runs. Measured: three calls with `a73afacfdb02` returned `0`; the full SHA returned `2`. If
you use it, pipe the SHA in — `--commit $(gh pr view N --json headRefOid --jq .headRefOid)` — rather
than retyping it. See "an empty result is not a finding" under the rules.

**Sandbox disk — this will evict you if you ignore it.** The agent workspace is a 10Gi EmptyDir
volume. The checkout is ~176M, so effectively all of it is Rust `target/`, and this workspace has no
`.cargo/config.toml` while the root `Cargo.toml` overrides only `[profile.release]` and
`[profile.ci]` — nothing lowers `debug` for the dev or test profiles. Full debug info is usually the
majority of target size, and `--all-targets --features db-tests` multiplies it. **Three** lanes have
been evicted mid-carve this way (`Usage of EmptyDir volume "workspace-data" exceeds the limit
"10Gi"`).

**All three evictions were self-inflicted at task creation, and the fix is one flag.** The repo
*does* size its workspace: the `moatless` entry in `.moatless/workspaces.json` sets a `resources`
block — `"storage": "60Gi"` today, `"20Gi"` when this was written, with explicit `requests` and
`limits` — and `docs/sandbox-architecture.md:752-774` documents that block as the answer to this
exact eviction for Rust-heavy repos. None of the evicted lanes were using it. A task created with
`moat tasks create --repo moatless` lands on a synthesized per-repository workspace — its id is
literally `ws_` + the repository id and its name is `null` — which carries no `resources` block at
all and therefore gets the deployment default. Tasks created with `--workspace moatless-global`
land on `ws_a7a61c35…`, which has the block. **Create carve lanes with
`--workspace moatless-global`.** The protocol below still stands: it costs nothing on a
correctly-sized workspace, and a lane can still find itself on a constrained one.

That workspace was named `Moatless` when this was written, and the entry's `key` is still
`moatless`. Do not pass `--workspace Moatless`: name lookup is a case-insensitive exact match that
errors only on ambiguity, so it now resolves silently to the manual `moatless` workspace
(`ws_688b4fb4…`), which sets only `storage` and is not the workspace these lanes were sized
against — no `requests`/`limits` means the deployment `K8S_RESOURCE_*` memory defaults, which is
the other half of the eviction above.

**A residual gap, measured and deliberately not fixed here.** Even the `moatless-global` workspace
does not move `CARGO_TARGET_DIR` off the workspace volume, and no workspace here declares storage
separate from it, so `target/` spends the same `storage` budget as the checkout. That makes the
number a larger ceiling rather than a different resource. Changing it touches
every task on that workspace, not just this program, which is why it was left alone. It is
**tracked in `docs/deferred-changes.md`** under "Test and CI structure", as "a sandbox's `target/`
spends the same `storage` budget as its checkout", with its anchor and entry condition; the
`CARGO_TARGET_DIR` row beside it is a different problem, the CI jobs' restored cache.

**Push early, and push broken.** This is the rule that matters most and the one this section was
missing through the first three evictions: it told you how to make eviction *less likely* and
nothing about making it *cheap*. As soon as the `git mv` and the new `Cargo.toml` skeleton exist —
before it compiles, before a gate runs — commit and push a WIP branch, and push again after every
meaningful step. The third eviction cost an entire carve because nothing had been pushed; the first
two cost minutes because something had. A branch that does not compile is not embarrassing, it is a
backup.

**Do not verify locally at full scale.** A workspace-wide `cargo test` or `clippy --all-targets` is
what fills the volume. Run targeted checks locally — `cargo check -p <the-new-crate>`, then
`cargo check -p moatless-backend` — and let CI carry `--all-targets` and the test run. CI is the
actual gate and it has the disk. This changes what you may *claim*, not what gets checked: say which
checks ran locally and which were left to CI, and never report a gate as passing on the assumption
that CI will pass it.

Export these before any cargo command:
```bash
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
```
`CARGO_INCREMENTAL=0` is not an afterthought beside the two debug settings — incremental artifacts
are gigabytes on a workspace this size, and they are pure rebuild-latency cache with no effect on
any output a gate reads.
They change nothing a gate measures — clippy lints, test outcomes and the census are identical; you
lose line numbers in backtraces. Then prefer `cargo check` over `cargo build`, check `du -sh target`
after every cargo invocation rather than between phases, and `cargo clean` past ~5G — do not wait
until it feels large, because the limit counts the whole workspace and not just `target/`. A cold
cache costs minutes; an eviction costs the sandbox. **If disk pressure forces you to skip a gate,
push and let CI run it — but report it as "deferred to CI", never as green.** A gate not run and a
gate that passed read identically.

**Formatting.** Do **not** run `cargo fmt`. The tree is not fmt-clean, CI has no `fmt --check`, and a
run rewrites ~194 files and collides with every concurrent lane. Match neighbours by hand. Never run
`rustfmt` on a file your edit did not already reformat. See the parked entry in the spec.

**Comments** state the current invariant. Change history goes in the PR body. A comment describing
where the code is *going* reads, a year later, as where it is.

## The rules, and what each cost to learn

These are the ones that repeatedly overturned confident conclusions. Every one of them was learned by
being wrong first.

**Read every occurrence; never grep the shape you expect.** Enumerating every `state` token found 11
ports where a single-line grep found 9 — and one of the two misses was code the same lane had written
*an hour earlier* in the same file. It defeats "I would notice my own code". Six independent findings
now rest on this.

**A grep that returns nothing is the most dangerous kind.** Absence of a spelling proves the spelling
absent, never the fact false. Negative claims ("there are no X", "this is the first X") need a
widened pattern *and* a read hit. This shipped a false claim into the plan once.

**Generalised: an empty result is not a finding — it is the absence of one, and the two are
indistinguishable at the point of reading.** The same shape bit this program three ways in a single
hour, and in every case the empty output was the *reassuring* reading:

| The empty output | Looks like | Actually was |
|---|---|---|
| `grep` for `#[expect]` returns nothing | the backend uses none | the multi-line attribute form was not matched (~19 files use it) |
| `gh run list --commit <short-sha>` returns 0 rows | no CI runs yet | abbreviated SHA; exit 0, no error; the full SHA returned 2 |
| `clippy` killed at a 2-minute tool cap, zero diagnostics | timed out, risk to defer | it was green in 1m26s once the tool budget was raised |

The habit that catches all three: before believing an empty result, make the command produce a
**non**-empty one you can recognise. Widen the pattern until something matches; run the query against
a case you know has rows; let the process finish once. A tool that was killed reported nothing, and
nothing reads as clean.

**A duplicate is proved by behaviour, never by a dependency list.** Two call sites with the same
ports are evidence of a shared *substrate*, not a shared *policy* — policy lives in the function name
and the doc comment. Collapsing github_pr's `api_routes` credential path into `auth().client_for_user()`
would have compiled and silently added a bot-token fallback to routes that deliberately fail closed.

**Placement before inversion.** "Does moving this make the edge legal?" precedes "what port inverts
it?" A trait that exists only to preserve a misplacement is the anti-pattern. Two of the largest
single wins (`adapters/routes.rs`, −42; the twelve handlers) were placements the spec had already
recorded.

**A bag is not a service.** A bag is ports passed *as a param* to narrow a signature, unpacked at the
far end. A service is constructed once, its **methods are the behaviour**, its ports are their
substrate. Width alone decides nothing — `RoutingService` is 10 ports and blessed. Both provider
routers arrived at this shape independently under the same constraint.

**Refuse the registry, transitively.** Never hand a provider something that *reaches* the aggregate
containing it. The test is not "is this the registry" but "does this transitively reach the aggregate
that contains me" — answered by **construction order, not band**. If a thing is constructed after
you, you cannot hold it (`state.rs:464` is the worked example).

**The registry launders an upward edge.** `Connector → Adapters → github_pr` is direction-legal at
every hop and the census cannot see it, because dyn dispatch through a band-8 aggregate is exactly
what a legal port looks like. The tell is the **argument**: naming a specific `AdapterKind` literal
below the provider band. Grepped and bounded — it was one production site, and removing it left the
whole capability dead.

**Annotation authority.** Prose we wrote, riding in the same document as measurements, gets read
later as measurement. Land a plan entry **with** the diff that witnesses it, never ahead of it. A
finding filed under the step that will consume it reads as *scheduled*, so nobody re-asks whether it
is available now.

**A well-formed argument is not evidence its inputs are true.** A rule applied faithfully to a false
premise produces a wrong result with a clean conscience. The plan is the worst place for it.

**Prefer the change you can verify by reading the diff.** When a behaviour change is free to avoid
and its justification is a reasoning chain rather than a test, avoid it. "No observable production
effect" is a conclusion from a model of production.

**Enumerate unmerged files with `git status`**, never from truncated rebase output, and never
`git add -A` into a conflicted tree.

## Out of scope / parked

Slack's retrofit to adapter-carries-ports (still has the 6-extractor shape); the `cargo fmt` sweep
(parked on measured facts — see spec); the stale `commitlint`
`scope-enum`; 59 Dependabot vulnerabilities on the default branch; PRs #422, #394, and **#379 (ignore)**.
The linear close-core delegation is a **hypothesis, not scheduled** — if walked, walk it cold.
