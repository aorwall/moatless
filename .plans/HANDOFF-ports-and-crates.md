# Handoff — backend ports and crates

> **For:** an agent starting this work with no prior conversation.
> **Spec:** `.plans/backend-ports-and-crates.md` is authoritative for *what* to build. This file is
> operational: where the work stands, how the lanes divide, and the rules that were learned the hard
> way and are not derivable from the code.
> **Updated:** 2026-08-09 · main at `3dc19c37f612`, census ceiling **0**. The first half is complete.

## What this program is

Two halves, in order:

1. **Make the ports singular** — remove every illegal reference where a lower-band module reaches up
   into a higher band. Measured by `scripts/crate-census-baseline.txt`, a ratchet. Started at **83**.
   **Now 0. This half is done.**
2. **Split the backend** — extract each band into its own crate. `moatless-loops` is the first and is
   **merged** (#466). Providers follow, then server last. **This half is where the work now is.**

The census only counts *production* reference edges (`CODE`/`ATTR`/`MACRO`). Doc paths and
`#[cfg(test)]` edges are excluded by design — see "test-context" below.

## Where it stands

**Main:** `3dc19c37f612`. **Ceiling: 0** — read from the baseline blob on `main`, not from a report.
There are no illegal production edges left in the repository.

What remains in the baseline file is 2 `[DOC]` intra-doc paths (`github_pr/adapter.rs:107`,
`telegram/adapter.rs:80` → `crate::compose`). These are **not Cargo edges** and are not counted;
they are cleaned as Step 8f carve work.

**Do not read 0 as "nothing left to do."** It means the *ports* half is finished. The ratchet now has
a new job: it stops the crate-splitting half from reintroducing what the first half removed. Every
carve from here is measured against 0, and the interesting failure is no longer "an edge remains" but
"an edge appeared and the census did not score it" — see the `external_crate` gap under Known gaps.

**All three lanes are closed.** History, because the PR numbers are the only durable record of why
each shape is what it is:

| Lane | Outcome |
|---|---|
| **linear** | **Done.** #456 (L1 webhook) → #461 (L2 api_routes) → #464 (reshape) → #465 (L3 issues) → #467 (L4 delivery+sessions). Zero `adapter-linear` edges. |
| **github_pr** | **Done.** #457 (handlers ingress), #458 (Connector + dead registry teardown), #460 (api_routes), #463 (shell_reconcile), then #469 — the four-edge router slice that took 4 → 0. |
| **loops carve** | **Done.** #466, the first crate this program split out. Landed illegal-neutral. |

## What to do next

In order. Nothing here is in flight; all three lanes stopped clean.

1. **Backfill `external_crate("moatless_loops")`** — one line in `bands.rs`, plus a regen. Ruled,
   measured inert, details under Known gaps. Do this **before** the next carve, not as part of it.
   Say in the PR body that it is measured-inert and closes an *unscored* path rather than fixing a
   counted edge, so that a later reader does not see a zero-diff tool change and revert it as a no-op.
2. **Carve the next crate.** Ask the census which one rather than picking from the band table by eye.
   It prints either `FRONTIER -> <crate>: READY now (no illegal outgoing; every crate it depends on
   is extracted).` or `FRONTIER -> <crate>: BLOCKED on <n> illegal outgoing.` (`main.rs:1081`), and
   the report's Tarjan SCC independently confirms it. Read the carve checklist first: the loops carve
   was sound on the graph and still went red five times on things the graph does not model.
3. **Providers, then `server` last.** That order is the spec's, not a preference.

The ratchet is at 0, so any carve that introduces an illegal edge fails immediately — which is the
point, and also why step 1 matters: an unscored edge fails nothing.

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
    **unscored** — so a future illegal edge into the carved crate is not reported as tolerated, it is
    not reported at all, and the census prints a 0 that is false. `loops` shipped without it (see
    Known gaps); `tasks` shipped without both at 10d and they were restored at 10e.
  A carve that omits the second arm still measures neutral on the day, because the only crates naming
  it are the ones that just moved. That is why it is easy to skip and why skipping it is invisible.
- Test-context edges are **not** ratcheted. Pricing them would fire on every legal in-crate test and
  create pressure to weaken tests to satisfy a counter.

## Known gaps — open, measured, deliberately not fixed in the PR that found them

Each of these was found with a measurement, not a suspicion, and each was kept out of the PR that
surfaced it. That separation is deliberate: a fix folded into the diff that tripped it is a fix
nobody reviews.

**`external_crate("moatless_loops")` is missing from `bands.rs`.** The loops carve landed
`floor_crate_of_dir("loops")` but not its pair, so it satisfied one of the two arms the carve rule
requires. **Measured inert:** adding the arm and regenerating produced a zero-diff baseline — ceiling
unchanged at the time, byte-identical. It is inert only because the sole crates naming
`moatless_loops::` today are the three backend facades, whose edges are legal or unscored either way.
The latent hole is a lower-band crate (`tasks`, `identity`, `workspaces`, `sandbox-*`) that later
gains a `moatless-loops` dependency and names it upward: that edge resolves to `None` and is never
counted. **Ruled: backfill it as its own one-line tool PR with a regen**, the way 10e restored
`tasks`. Not as part of a carve — an instrument must be fixed *before* the thing it measures moves,
or the first carve that exercises it changes the instrument and the subject in one step and there is
no clean before.

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

**The six gates.** A subset gate reporting "clean" is the single most repeated failure here. Run all
six before every push:

```
./scripts/check-plan-assertions.sh
./scripts/check-crate-census.sh
./scripts/check-appstate-composition-root.sh
./scripts/check-bindings-adapters-baseline.sh
./scripts/check-dao-composition-root.sh
./scripts/check-pr-title.sh
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
(parked on measured facts — see spec); making the pr-title check required; the stale `commitlint`
`scope-enum`; 59 Dependabot vulnerabilities on the default branch; PRs #422, #394, and **#379 (ignore)**.
The linear close-core delegation is a **hypothesis, not scheduled** — if walked, walk it cold.
