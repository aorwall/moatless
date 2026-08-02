# T3 Code integration: start here

Orientation for anyone picking up the T3 Code ↔ Moatless integration. Read this
first, then the documents below in order. Roughly an hour to be ready to plan.

## Check out both repositories

The work spans two repos, both on a branch named `moatless-adapter`:

```bash
git clone https://github.com/soaplabs/moatless.git
cd moatless && git checkout moatless-adapter        # ef4d860

git clone https://github.com/soaplabs/t3code.git
cd t3code && git checkout moatless-adapter          # d67a67a
```

Most of the planning material is on the **T3 Code** side. Most of the code written
so far is on the **Moatless** side. You need both checked out.

## Read in this order

**1. `moatless/CONTEXT.md`** — the domain glossary. The `T3 Code — always qualified`
section defines the four terms that collide (`T3 Thread`, `T3 Project`,
`T3 Environment`, `UI RPC`). ~10 min.

**2. `t3code/docs/reference/client-server-contract.md`** — what T3 Code promises its
client: 23 HTTP endpoints, 70 WebSocket methods, the subscription contract, and the
20-member command union. This is the surface the adapter has to satisfy. ~20 min.

**3. `t3code/docs/reference/moatless-concept-map.md`** — the feature-by-feature walk
of that contract against this backend, marking each surface equivalent (`=`),
superficially similar (`≈`), or absent (`∅`). **§6 "Things that are _kind of_ the
same"** is the highest-value section in either repo — read it even if you skim the
tables. ~30 min.

**4. `moatless/docs/t3-code-current-state.md`** — what the Moatless backend
currently implements: the opt-in compatibility routes, shell snapshot, listing
feed, auth boundary, and known gaps.

**5. `moatless/docs/t3-code-domain-mapping.md`** — the Moatless-side mapping
details: which vocabulary wins, the two renames to refuse, and how shell fields
are projected.

**6. `t3code/.plans/moatless-convergence.md`** — the four-move proposal for changing
*both* sides so the adapter shrinks from a simulation layer to a translation layer.
Explicitly not scheduled. Read after the concept map; it assumes that vocabulary.

**7. `t3code/.plans/moatless-adapter.md`** — the original full adapter spec (~990
lines). Note that concept-map §8 lists corrections to it, so read the corrections
alongside it rather than taking it standalone.

## Then read the code

- **`moatless/crates/t3code/src/lib.rs`** — the compatibility surface. Discovery,
  cookie auth, `/api/orchestration/shell`, WebSocket upgrade, Effect-RPC
  envelopes, shell projection, and shell subscription replay are wired end to end.
- **`moatless/docs/runbooks/t3-code-local-development.md`** — how to actually run the
  two together locally.

## Where to start planning

The Moatless backend shell is populated. Remaining work is mainly command
coverage, T3-side remote auth UX, and the contract mismatch for environment-scoped
operations against per-Task Sandboxes.

1. **The event cursor.** T3's client is a sequence-fold client; Moatless has no
   monotonic sequence. `task_events.id` is the obvious candidate and is **not safe
   as-is** — a `bigserial` is assigned at `INSERT` and visible at `COMMIT`, so a
   naive `id > N` poll silently skips events. Three options in
   convergence §3.5. *This is the genuinely hard one — settle it first.*

2. ~~**What a T3 Project maps to.**~~ **Settled: Workspace** (`projectId ↔
   workspaces.id`). This supersedes the Project→Repository row in concept-map §1 —
   treat that row as stale when reading it. The field-by-field mapping and the
   `workspaceRoot`-is-an-identity hazard are in
   [the domain-mapping doc](./t3-code-domain-mapping.md).

3. **Environment granularity.** Convergence §2.2 recommends Option B — the
   deployment is the one T3 environment, sandboxes are thread-scoped state — and
   notes this is a *prerequisite* for the later moves, not a consequence. Confirm it
   before building on it.

Everything else (turn status enums, settled/snoozed, shell pagination) is
mechanical once these three are fixed.
