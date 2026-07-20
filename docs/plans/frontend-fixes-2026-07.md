# Frontend fixes: dead code, reconnect staleness, lint debt, list virtualization

Four frontend workstreams from a July 2026 architecture review, ordered by recommended
execution. They are independent — each can ship as its own PR.

| # | Workstream | Size | Risk |
|---|------------|------|------|
| 1 | Remove `@moatless/diff-viewer` | S | Low |
| 2 | SSE reconnect cache invalidation | S | Low |
| 3 | Ratcheted Biome lint baselines | M | Low |
| 4 | Virtualize the message history | L | Medium |

---

## 1. Remove `@moatless/diff-viewer`

### Problem

`packages/diff-viewer` is a full hand-rolled diff-rendering package (diff parsing,
word-diff, gutter/line/card components) built on `diff` + `highlight.js`, but the app's
actual diff UI is `@pierre/diffs` / `@pierre/trees`. The only live consumer is
`apps/frontend/src/features/files/components/markdown-preview.tsx:1`, which imports just
three syntax-highlighting exports:

```ts
import { type HighlightToken, highlightLine, SyntaxToken } from '@moatless/diff-viewer'
```

Everything else in the package (`components/diff-*.tsx`, `hidden-chunk.tsx`,
`utils/diff-parser.ts`, `utils/word-diff.ts`, …) is dead surface that still costs
maintenance, `postinstall` build time (`turbo run build --filter='./packages/*'`), and
confusion about which diff stack is canonical.

### Plan

1. **Relocate the three used exports** into the frontend, next to their one consumer:
   - `packages/diff-viewer/src/utils/syntax-highlight.ts` → `apps/frontend/src/features/files/lib/syntax-highlight.ts`
   - `packages/diff-viewer/src/components/syntax-token.tsx` → `apps/frontend/src/features/files/lib/syntax-token.tsx`
   - The `HighlightToken` type (from `packages/diff-viewer/src/types/`) moves with them.
   - Keep the deliberately small hljs language registration subset (js/ts/py/json/css/html/bash/yaml/md/go/rust/java/sql) — that bundle-size decision is worth preserving. `highlight.js` becomes a direct `apps/frontend` dependency (it already reaches the bundle transitively today, so net bundle impact is zero).
2. **Update the import** in `markdown-preview.tsx` and in `markdown-preview.unit.test.tsx` (note: this test is excluded from `test:unit` for memory reasons — run it explicitly via `bunx vitest run src/features/files/components/__tests__/markdown-preview.unit.test.tsx`).
3. **Delete `packages/diff-viewer/`** and remove references:
   - `apps/frontend/package.json` → drop `"@moatless/diff-viewer": "workspace:*"`, add `highlight.js`.
   - `apps/frontend/tsconfig.json` → drop the two path aliases (lines ~30–31) and the project reference (line ~38).
   - `biome.json` → remove from the scanned-paths list if present.
   - Check `apps/frontend/vite.config.ts` for any alias entries.
4. **Port any package-level tests** for `syntax-highlight.ts` into `apps/frontend/src/features/files/lib/__tests__/`; delete the rest with the package.

### Verification

`bun install && bun run type:check && bun run biome:check && bun run test:unit`, plus the
excluded markdown-preview test explicitly. Manually open a task file view with a markdown
file containing fenced code blocks and confirm highlighting renders.

### Non-goals

Do not try to migrate `markdown-preview.tsx` to a different highlighter in the same PR —
this is a pure code-motion/deletion change.

---

## 2. SSE reconnect cache invalidation

### Problem

`apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx` is the only
live-data channel: SSE events patch the TanStack Query cache directly. On disconnect it
reconnects with exponential backoff and shows a "Reconnected to server" toast
(`wasReconnecting`, ~line 455) — **but it never refetches anything**. Every event emitted
during the gap is silently lost, so the cache (task status, messages, sandbox status) stays
stale until the user navigates or an unrelated invalidation fires. The backend has no
`Last-Event-ID` replay (verified: no such handling in `backend/src/sse/`), so the client
must treat a reconnect as "anything may have changed" and invalidate — scoped to *active*
(currently observed) queries so a reconnect never turns into a refetch storm.

### Plan

1. In the connect-success path (where `wasReconnecting` is computed), before resetting
   `reconnectAttemptsRef` and toasting, add:

   ```ts
   if (wasReconnecting) {
     // Events emitted while disconnected are gone (no server-side replay),
     // so anything observed on screen may be stale. refetchType: 'active'
     // limits this to mounted queries — dormant cache entries refetch
     // naturally on next observation via their own staleness.
     void queryClient.invalidateQueries({ refetchType: 'active' })
   }
   ```

   Note there is **no `queryKey` filter**: the namespaces in
   `src/lib/api/query-keys.ts` do not share a common root (`queryKeys.all` is
   `['swarm']`, but `queryKeys.tasks.all` is `['tasks']`, `auth` is `['auth']`, etc.), so
   no single prefix covers the caches that matter. Invalidating everything is correct
   here — `refetchType: 'active'` means only mounted queries actually refetch; dormant
   entries are merely marked stale and refetch on next observation.
2. **Exclude nothing initially.** The targeted-invalidation logic in
   `features/events/utils/event-handlers.ts` exists to avoid refetch storms from
   *high-frequency events*; a reconnect is rare, so a one-shot active-only refetch is
   acceptable. If task-list refetch proves disruptive, narrow with a `predicate` later.
3. **Guard against reconnect loops**: only fire when the connection was actually
   established (the code path already distinguishes this — reuse `wasReconnecting`), not on
   every retry attempt.
4. Optional follow-up (separate PR, backend): add monotonic event ids + `Last-Event-ID`
   replay to `backend/src/sse/` so short gaps replay instead of refetching. Not required
   for correctness once step 1 lands.

### Verification

- Extend `features/events/contexts/__tests__/global-event-stream-provider.unit.test.tsx`
  (the existing home for stream lifecycle tests — note the `.integration.test.tsx` name
  referenced in `apps/frontend/package.json`'s test scripts no longer exists): simulate
  connect → disconnect → reconnect and assert `invalidateQueries` is called with
  `refetchType: 'active'` and no `queryKey` filter, and that it is *not* called on
  initial connect.
- Manual: open a task, kill the backend (`moat docker` / compose), send a message via
  `moat send` from another session while the UI is disconnected, restart backend, confirm
  the message appears after the "Reconnected" toast without a page refresh.

---

## 3. Ratcheted Biome lint baselines

### Problem

Three valuable correctness rules are hard-off in `biome.json` (lines 51–54) because the
existing violation count makes enabling them all-or-nothing:

```json
"nursery": {
  "noFloatingPromises": "off",
  "noMisusedPromises": "off",
  "useExhaustiveSwitchCases": "off"
}
```

The fix is a **ratchet**: hard-block new violations while grandfathering an enumerated
per-file baseline, which can only shrink. Biome has no native baseline feature, but our
`biome:check` already runs with `--diagnostic-level=error`, which means **warn-level
diagnostics are printed but don't fail** — exactly the hook a ratchet script needs.

### Plan

1. **Flip the three rules from `"off"` to `"warn"`** in `biome.json`. `bun run biome:check`
   behavior is unchanged (it only fails on errors); editors start showing the squiggles
   immediately, which is itself a win.
2. **Add `scripts/biome-ratchet.ts`** (bun script, no new deps):
   - Runs `biome lint --reporter=json apps/frontend/src packages` and filters diagnostics
     to a `RATCHET_RULES` allow-list (the three rules above; extensible later).
   - Aggregates counts per `(file, rule)` and compares against a checked-in
     `biome-baseline.json` (same shape: `{ "path": { "rule": count } }`).
   - **Fails** if any `(file, rule)` count exceeds baseline, printing the new diagnostics.
   - **Warns** (and suggests running with `--update`) if a count dropped — keeping the
     baseline honest so fixed debt can't regress back.
   - `--update` rewrites the baseline from current counts, but refuses to *increase* any
     entry (updates only shrink; increases require editing the file in review, on purpose).
   - A file absent from the baseline has an implicit count of 0 — new files must be clean.
3. **Generate the initial baseline** (`bun scripts/biome-ratchet.ts --update --init`) and
   commit it. Expect the bulk to be `noFloatingPromises` in event handlers and mutation
   call sites.
4. **Wire into CI and turbo**: add `"lint:ratchet": "bun scripts/biome-ratchet.ts"` to root
   `package.json`, add it to the `check:turbo` pipeline in `turbo.json` (cacheable, inputs =
   source + `biome.json` + baseline), and to the frontend CI workflow next to
   `biome:check`.
5. **Document** in CLAUDE.md's Mandatory Workflow section: violations of ratcheted rules in
   touched files should be fixed, not baselined.

### Verification

- Unit-test the ratchet's compare logic (pure function: `(baseline, current) → verdict`)
  in `scripts/__tests__/`.
- Prove the ratchet bites: add a deliberate floating promise in a scratch branch, confirm
  `lint:ratchet` fails; remove one existing violation, confirm it reports the shrink.

### Burn-down (optional, ongoing)

Once in place, the baseline doubles as a work queue. `noFloatingPromises` fixes in the
event-stream provider and `use-tasks.ts` are the highest-value targets — those are the
places where a swallowed rejection actually hides bugs.

---

## 4. Virtualize the message history

### Problem

`features/tasks/components/chat/message-history/message-history.tsx` renders the full
message list as plain DOM inside a scroll container, with substantial hand-rolled scroll
management already present (~lines 190–310): stick-to-bottom, a scroll-down button,
`fetchPreviousPage` on upward scroll with scroll-position restoration via captured
metrics, and re-arm logic to dodge queued-scroll-event races. Rendering cost per message is
high (markdown via `streamdown`, tool-call cards, per-message `ErrorBoundary`), so
long-running tasks — exactly our showcase scenario — degrade: every SSE-driven update
re-renders and re-lays-out an unbounded list. Virtualizing keeps DOM size and layout cost
proportional to the viewport instead of the conversation length.

### Library decision

Recommendation: **`virtua`** (`VList`), with `@tanstack/react-virtual` as the fallback.

- `virtua` is purpose-built for chat UIs: a `shift` prop for prepend without scroll jump
  (replaces our hand-rolled anchor-restore), `reverse`-friendly, handles dynamic item
  heights without manual measurement, ~3 kB, MIT.
- `@tanstack/react-virtual` fits our TanStack-everything stack but is headless: we would
  keep and adapt *all* the existing anchoring code around it. Choose it only if `virtua`'s
  spike (step 1) fails.
- `@legendapp/list` is React-Native-first; no advantage for a web-only app.

### Plan

1. **Spike behind a flag** (½ day): wrap the existing `RenderItem[]` list in `<VList shift={isPrepending}>`
   guarded by `localStorage.moatlessVirtualList === 'true'`, with the three behaviors
   wired minimally. Kill criteria: broken stick-to-bottom during streaming, visible jumps
   on prepend, or tool-call cards with collapsing/expanding height thrashing the
   viewport. If it fails, rerun the spike with `@tanstack/react-virtual` + existing
   anchor code.
2. **Preserve the current contract**, which is already cleanly expressed in the component:
   - *Items*: the `RenderItem` union (`message` | `cluster`) stays the virtualization
     unit — do not virtualize inside clusters.
   - *Stick-to-bottom*: replace the manual `scrollTop = scrollHeight` effect with
     `VList`'s `scrollToIndex(items.length - 1)` driven by the same "user is at bottom"
     state; keep the scroll-down button logic reading `VList`'s scroll offset.
   - *Reverse infinite scroll*: on `fetchPreviousPage` success, set `shift` for the
     render that prepends — this deletes the `prependAnchorRef` metric
     capture/restore code and the re-arm workaround (~80 lines of the trickiest logic in
     the component).
   - *Per-item `ErrorBoundary`* and `LastAssistantActions` pinning stay as-is (they are
     per-item render concerns, orthogonal to virtualization).
3. **Extract scroll logic as `message-history-scroll.ts`**: pure functions for "is
   at bottom", "should re-arm prev-page trigger", "should show scroll-down button", each
   unit-tested next to the component. This is the part of the component that has
   historically raced; tests here are the regression net for the migration, and the
   extracted-pure-logic pattern is worth adopting for other heavy components afterwards.
4. **Ship**: flip the flag default on, keep the non-virtualized path for one release
   behind the same flag as an escape hatch, then delete it.

### Verification

- Unit: the new `message-history-scroll.ts` module (jsdom can't measure real layout —
  logic tests + E2E cover the split).
- E2E (`tests/ui/`): extend `multi-message.spec.ts` — send enough messages to exceed the
  viewport, assert (a) newest message visible while streaming, (b) scrolling up and
  triggering a previous-page load does not move the viewport-anchor message, (c)
  scroll-down button appears when scrolled up and works.
- Perf check: with a 500+ message task (create via script against a docker-mode backend),
  compare React DevTools commit times and DOM node count before/after; expect node count
  to drop from O(messages) to O(viewport).

### Risks

- Dynamic item heights (expanding tool-call cards, streaming markdown growth) are the main
  correctness risk — this is what the spike exists to test.
- `LastAssistantActions` and the thinking indicator render after the last item; ensure
  they live *inside* the virtualized list as a footer item (or `VList` children tail) so
  stick-to-bottom accounts for their height.
- Kiosk layout (`kiosk-task-layout.tsx`) shares this component — include it in manual QA.
