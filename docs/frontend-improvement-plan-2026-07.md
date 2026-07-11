# Frontend Improvement Plan — July 2026

Follow-up to the [frontend audit](./frontend-audit-2026-07.md) and PR #250 (top-priority
fixes: CSP headers, bundle splitting, error boundaries, auth returnTo, SSE hardening,
Tailwind v4 migration, a11y labels, test un-skipping) plus the subsequent type/lint
hardening (`verbatimModuleSyntax`, `noUncheckedIndexedAccess`, exhaustive event switches).

This plan covers the three remaining code-quality tracks:

- **Track A — API-boundary type safety** (audit: typescript 6/10, data-fetching 7/10)
- **Track B — Mechanical state & performance cleanups** (audit: state 6/10, performance 5/10)
- **Track C — Larger refactors** (audit: components 6/10, styling 6/10)

Explicitly **out of scope** here (tracked separately): the test-coverage campaign + MSW
adoption, error-monitoring SDK integration, and production CSP verification.

**Dependency upgrades** (and the new features / best practices they unlock) are now covered in
a dedicated companion: [frontend dependency upgrades](./frontend-dependency-upgrades-2026-07.md).
It also flags two latent version-drift **bugs** (Vitest core v3 vs. `@vitest/*` v4;
`@storybook/test@8` alongside Storybook 10) and threads several upgrades into the tracks below —
notably C2 (React Compiler 1.0), A4 (`queryOptions`), B6 (router `autoCodeSplitting`), and C3
(Tailwind v4 `@theme inline` semantic tokens).

All file references verified against `main` @ `a383052`. Paths are relative to
`apps/frontend/` unless noted.

> **Update — post-merge with `main` @ `6c8cbf6` (2026-07-11).** The task-page
> composability refactor (#258/#259/#260, see
> [task-page-composability](./plans/task-page-composability.md)) landed and moves several
> references below:
> - **C4 (chat layering) is DONE** — `lib/components/chat/*` moved into
>   `features/tasks/components/chat/`; `lib` no longer imports feature internals. The C1
>   "layering violation" note is likewise resolved.
> - **C1**: `task-page.tsx` (426 lines) was **dissolved** into `layouts/` + `panels/` +
>   `contexts/` — that row is obsolete. `assistant-message.tsx` (831) now lives under
>   `features/tasks/components/chat/message-history/`; `preview.tsx` (781) and
>   `chat-container.tsx` (751, now `features/tasks/components/chat-view/`) are unchanged and
>   still oversized.
> - **B1/B2**: the two `task-page-layout.tsx` provider sites are **gone** (file deleted). The
>   three new providers (`sandbox-events-provider`, `task-composer-provider`,
>   `task-panel-controls`) already `useMemo` their value — no B1 work needed there — but they
>   add three `.Provider` sites to the B2 codemod. Lists below updated.
> - **B4**: the shared `lib/api/task-sandbox-queries.ts` now sets `staleTime: 5000` +
>   `refetchOnWindowFocus: false` (the staleTime sub-point is **done**); the per-row mount
>   burst still stands.
> - **B5**: the `task-page.tsx` effects moved into the new providers as legitimate
>   SSE-accumulation effects — that sub-item is resolved; `profile-configuration-section` and
>   `canvas-search` are unchanged and still apply.
>
> Inline references below are corrected accordingly. The dependency-upgrade companion is
> unaffected (its file references still hold; if anything the new
> `sandbox-events-provider` SSE effect strengthens the React 19.2 `useEffectEvent`/`<Activity>`
> case in its D2).

---

## Track A — API-boundary type safety

**Goal:** every value entering the app from the network is typed by construction (generated
types or schema parsing), not by assertion. After the tsconfig hardening, the remaining gap
is a handful of casts at the API boundary and query-key drift.

### A1. Remove per-callsite request casts (high value, small)

Three production casts silence real mismatches between form/Zod types and generated
`@moatless/api-client` request types:

| Location | Cast | Fix |
|---|---|---|
| `src/features/tasks/api/tasks.ts:121` | `payload as ApiSendMessageRequest` | Align the local `SendMessageRequest` shape with the generated type; if a field genuinely differs, map it explicitly instead of casting the whole object. |
| `src/features/repositories/components/sections/repository-config-edit-section.tsx:130` | `payload as UpdateRepositoryRequest` | Same: build the payload with the generated type as the declared type so excess/missing fields fail at compile time. |
| `src/features/repositories/components/sections/repository-config-create-section.tsx:159` | `value as GitProvider` | Replace with a type guard or a `Select` typed over the enum values (`satisfies readonly GitProvider[]` on the options array). |

**Pattern to adopt:** declare payload objects `const payload: UpdateRepositoryRequest = {...}`
(errors surface at construction) or use `satisfies`. Never cast a structurally-different
object into a request type.

### A2. Contain the trust-boundary cast in the API client (small, documentation-heavy)

`src/lib/api/client.ts:84,87`:

```ts
if (response.status === 204) return null as unknown as T
return (await response.json()) as unknown as T
```

A generic `request<T>` cannot avoid one unchecked assertion — this is the designated trust
boundary. Actions:

1. Keep a **single** assertion, but make the 204 case honest: `request<T>`'s callers that
   expect `void`/`null` should use a dedicated `requestVoid()` (or `T = null` overload) so
   `null as unknown as T` can't leak a `null` into a non-nullable `T`.
2. Add a comment marking this as the app's only sanctioned network-boundary cast.
3. (Optional, later) For high-risk endpoints, layer Zod `parse()` in the feature API module
   — the schemas already exist for several features. Do not do this wholesale; runtime
   validation everywhere duplicates the generated OpenAPI types.

### A3. Centralize the remaining inline query keys (small, mechanical)

`.claude/rules/frontend-conventions.md` mandates all keys come from
`src/lib/api/query-keys.ts`, but these are still inline:

- `['auth', 'session']` — 7 occurrences: `src/features/auth/hooks/use-auth.ts:21`,
  `src/features/settings/hooks/use-github-connection.ts:60,81,102,120`,
  `src/features/settings/hooks/use-profile.ts:11`, `src/routes/admin.tsx:12`
- `['figma', 'imports', importId]` — `src/features/figma/hooks/use-figma-import.ts:28`
- `['github-oauth-url']` — `src/features/settings/hooks/use-github-connection.ts:32`
- `['figma-oauth-url']` — `src/features/settings/hooks/use-figma.ts:28`
- File-tree/content extensions built ad hoc on top of `queryKeys.tasks.files(taskId)` —
  `src/features/files/hooks/use-file-tree.ts:21,27`, `use-file-content.ts:12`

Add `queryKeys.auth.session()`, `queryKeys.figma.*`, `queryKeys.oauth.*`, and
`queryKeys.tasks.fileTree(taskId, mode)` / `fileContent(taskId, path, withChanges)`
factories, then replace the literals. The duplicated `['auth', 'session']` is the risky
one — a typo in any of the 7 sites silently breaks session invalidation.

### A4. Adopt `queryOptions()` for shared queries (medium, incremental)

There is currently **zero** `queryOptions()` usage. The TanStack Query v5 pattern
colocates `queryKey` + `queryFn` (+ `staleTime`) in one typed unit and gives typed
`getQueryData` / `ensureQueryData` for free — which matters because the codebase reads
and patches caches imperatively (route loaders, `global-event-stream-provider.tsx`'s
`applyTaskEventUpdate`).

Incremental migration, highest-traffic first:

1. `tasksQueries.detail(id)` / `list(filters)` / `sandboxStatus(id)` — used by route
   loaders (`src/routes/tasks/$task-id/index.tsx`), the SSE provider, and hooks. The
   composability refactor already extracted `src/lib/api/task-sandbox-queries.ts`
   (`useTaskSandboxStatusQuery`, still inline `useQuery`) — the natural first `queryOptions()`
   conversion, and it's shared by both `features/tasks` and `features/files`.
2. `authQueries.session()` — pairs with A3.
3. New queries use `queryOptions()` from day one (add to
   `.claude/rules/frontend-conventions.md` once the pattern lands).

**Acceptance for Track A:** no `as`-casts of request/response objects outside
`client.ts`; `grep "queryKey: \['" src` (excluding `query-keys.ts`) returns nothing;
route loader and SSE cache access is typed via `queryOptions`.

---

## Track B — Mechanical state & performance cleanups

**Goal:** burn down the confirmed audit findings that are safe, local, and need no design
decisions. Each item is independently shippable; together they fit in one or two small PRs.

### B1. Memoize unstable context values (trivial)

Every consumer of these contexts re-renders on every provider render because the `value`
object is recreated inline:

- `src/lib/contexts/feature-flags-context.tsx:76` — inline
  `{ flags, isLoading, isEnabled, getSlackConfig }`
- `src/lib/components/theme-provider.tsx:52` — plain object (`const value = { … }`), not memoized
- `src/lib/components/ui/sidebar-dialog.tsx:58` — inline
  `{ isMobile, mobileSidebarOpen, setMobileSidebarOpen }`

(The `task-page-layout.tsx` sites in the original plan are gone — the file was deleted by the
composability refactor.)

Wrap in `useMemo` (and callbacks in `useCallback`). `task-ui-provider`, `task-data-provider`,
`compose-task-provider`, `sidebar.tsx`, and the three new providers
(`sandbox-events-provider`, `task-composer-provider`, `task-panel-controls`) already do this —
copy their pattern. Note: if C2 (React Compiler) ships, this becomes redundant — do it anyway;
it's minutes of work and the compiler adoption is not guaranteed.

### B2. React 19 `<Context value>` codemod (cosmetic, ride-along with B1)

Eleven app `.Provider` usages remain (`global-event-stream-provider.tsx:683`,
`task-data-provider.tsx:71`, `task-ui-provider.tsx:71`, `compose-task-provider.tsx:94`,
`sandbox-events-provider.tsx:132`, `task-composer-provider.tsx:118`,
`task-panel-controls.tsx:79`, `feature-flags-context.tsx:76`, `theme-provider.tsx:63`,
`sidebar.tsx:201`, `sidebar-dialog.tsx:58`). React 19 supports `<MyContext value={...}>`
directly and deprecates `.Provider`. Pure rename — fold into the B1 PR. (Exclude
`tooltip.tsx`'s `TooltipPrimitive.Provider` — that's a Radix primitive, not a React context.)

### B3. Hoist the react-markdown `components` object (trivial)

`src/features/files/components/markdown-preview.tsx:111` passes an inline `components={{...}}`
object, defeating react-markdown's memoization on every parent render. Hoist to module scope
(it closes over nothing that changes per render — verify, extract any dynamic piece via a
prop-driven component).

### B4. Kill the dashboard per-row sandbox-status burst (small, measurable)

`src/features/tasks/components/dashboard/task-table.tsx:48-50`: `TaskStatusCell` calls
`useSandboxStatus(task.taskId, task)` per row → a dashboard with N visible tasks fires N
`GET /sandbox/status` requests on mount (only `closed` tasks are excluded).

Options, in order of preference:

1. **Derive from data already on hand.** The task list response plus SSE patches
   (`applyTaskEventUpdate` already writes sandbox status into the detail cache) cover the
   common display states. Render from `task` fields + any cached
   `queryKeys.tasks.sandboxStatus(id)` entry via `getQueryData` — no new requests from the
   table; the cell upgrades to live data when the cache is populated by the task page or SSE.
2. If per-row freshness is truly required, add a batched `GET /sandbox/status?taskIds=…`
   endpoint (backend change — bigger scope, defer unless (1) proves insufficient).

~~Also set an explicit `staleTime` on the sandbox-status query so navigating dashboard →
task → dashboard doesn't refire everything.~~ **Done** by the composability refactor: the
shared `src/lib/api/task-sandbox-queries.ts` now sets `staleTime: 5000`,
`refetchInterval: false`, and `refetchOnWindowFocus: false` (SSE-driven). The remaining B4
work is only the mount-time burst — `task-table.tsx:49`'s `TaskStatusCell` still calls
`useSandboxStatus` per row (option 1 above, "derive from data on hand", is still open).

### B5. Replace `useEffect`+`setState` derivations (small each)

Violations of the "derive during render / don't sync state in effects" rule:

- `src/features/settings/components/profile-configuration-section.tsx:23` — effect copies
  session data into `name`/`email` state. Fix: initialize form state from the loaded
  session (render the form only when session data exists and seed `useState` initializers,
  or use a `key={user.id}` remount). Removes the flash-of-empty-form and the double render.
- ~~`src/features/tasks/components/task-page.tsx:117,230` — effects syncing preview
  path/context items~~ — **moot**: `task-page.tsx` was dissolved by the composability refactor;
  this state now lives in `contexts/sandbox-events-provider.tsx` (SSE event accumulation) and
  `contexts/task-composer-provider.tsx` (context items), where the effects are legitimate
  subscription/accumulation, not prop→state mirroring. No action.
- `src/features/figma/components/canvas-search.tsx:120,128` — debounce implemented as
  effect-chained state. Fix: `useDeferredValue` for the search term (React 19 handles this
  natively) or a self-contained `useDebouncedValue` hook; drop the intermediate state.

### B6. Route-level code splitting for cold routes (small)

No `.lazy.tsx` files exist — every route ships in the entry chunk. PR #250 split the
heaviest dependency (file explorer + wasm) but route-level splitting is still free wins.
Convert low-traffic routes to TanStack Router's `createLazyFileRoute`:

- `src/routes/admin.tsx` + `src/routes/admin/*`
- `src/routes/figma-test.tsx` (dev/test page — should not be in the entry bundle at all)
- `src/routes/settings.tsx` + settings feature components
- `src/routes/secrets.tsx`, `src/routes/sandbox.tsx` + `src/routes/sandbox/*`

Keep `index`/`dashboard`/`tasks` eager (first paint path). Verify with
`bun run build` chunk output before/after.

**Acceptance for Track B:** no inline context values; no `useEffect` whose only job is
`setState` from props/query data in the three files above; dashboard mount issues 0
sandbox-status requests (network tab); admin/settings/figma-test absent from the entry
chunk.

---

## Track C — Larger refactors

**Goal:** structural improvements that need their own branches and review cycles. Do these
opportunistically or as dedicated efforts — not as drive-by changes.

### C1. Decompose the God components (medium, per-component)

| File | Lines | Suggested seams |
|---|---|---|
| `src/features/tasks/components/chat/message-history/assistant-message.tsx` | 831 | Per-block renderers already implicit: tool-use, tool-result, thinking, text/markdown sections → one component each; extract the block-dispatch switch into a registry map. |
| `src/features/preview/components/preview.tsx` | 781 | Split: URL/route state management hook, toolbar, iframe host + load/error lifecycle, device-frame chrome. |
| `src/features/tasks/components/chat-view/chat-container.tsx` | 751 | Extract: scroll-position/anchoring logic into a hook, message-send orchestration into a hook, keep the component as composition. The composability refactor moved wiring into panels/providers but did **not** slim this file — still the largest in the feature. |
| ~~`src/features/tasks/components/task-page.tsx`~~ | ~~426~~ | **Done** — dissolved into `layouts/` (55-line `task-page.tsx` + 3 layout shells), `panels/`, and `contexts/` by the composability refactor. |

Rules of engagement: one component per PR; extraction only (no behavior change); the
existing tests (and any added in the test campaign) must pass unmodified. The **layering
violation** the audit flagged (`lib/components/chat` reaching into `features/tasks`) is
**already resolved** — the composability refactor moved all chat UI into
`features/tasks/components/chat/`, so `lib` no longer imports feature internals (see C4).

### C2. Evaluate React Compiler (medium, own branch, measure first)

React 19 + Vite makes this a config-level experiment: add `babel-plugin-react-compiler`
to the Vite React plugin on a branch.

Plan:
1. Run the compiler's compatibility check (`npx react-compiler-healthcheck`) and fix
   flagged rule violations (mutation-during-render etc.).
2. Enable on a branch; run the full suite + the Playwright smoke script; profile the task
   page and dashboard with React DevTools before/after (re-render counts while an SSE
   stream is active are the key metric — this app re-renders on every event).
3. Ship behind a comparison: if re-render counts drop materially with no regressions,
   adopt; otherwise document and close.

If adopted, most manual `useMemo`/`useCallback`/`memo()` (including B1) becomes
optimization the compiler owns — but don't remove existing memoization in the same PR.

### C3. Semantic status-color tokens (small-medium, mechanical after design)

Current state: **41** hardcoded palette classes (`text-red-500`, `text-green-500`,
`bg-red-*`, `text-yellow-*`) and ~**100** arbitrary values (`[NNpx]`, `[#hex]`) in
production `tsx`. These bypass theming — dark mode and any future rebrand break silently.

1. Define semantic tokens in `src/lib/styles/globals.css` under `@theme`:
   `--color-success`, `--color-warning`, `--color-destructive` (exists), `--color-info`,
   plus status-specific ones if needed (`--color-status-running`, etc.), with dark-mode
   values.
2. Migrate the 41 palette-class sites (mostly status icons/badges — task status, sandbox
   state, diff added/removed).
3. Sweep the arbitrary values: promote repeated ones (spacing, z-index, sizes) to tokens;
   leave true one-offs.
4. Optional guard: a Biome/grep CI check rejecting new `text-red-*`-style classes outside
   `ui/`.

### C4. Chat component layering — ✅ DONE (composability refactor, #258–#260)

`lib/components/chat/*` was moved wholesale into `features/tasks/components/chat/`, so the
shared `lib` layer no longer imports feature internals. Verified: no remaining
`lib/components/chat` imports and the directory no longer exists. The composability plan also
adds a Biome `noRestrictedImports` guard (tasks-as-composition-feature) to prevent regressions
— see [task-page-composability](./plans/task-page-composability.md) §"Phase 4".

---

## Suggested sequencing

| Order | Work | Size | Commit scope |
|---|---|---|---|
| 1 | A1 + A2 + A3 (casts + query keys) | 1 small PR | `refactor(utils):` / `refactor(hooks):` |
| 2 | B1 + B2 + B3 (memoization + Provider codemod + markdown hoist) | 1 small PR | `refactor(components):` |
| 3 | B4 + B5 (status burst + effect derivations) | 1 PR | `refactor(components):` |
| 4 | B6 (route splitting) | 1 small PR | `refactor(routes):` |
| 5 | A4 (`queryOptions` migration) | 1–2 PRs, incremental | `refactor(hooks):` |
| 6 | C3 (color tokens) | 1 PR after token naming is agreed | `refactor(styles):` |
| 7 | C1/C4 (component decomposition) | 1 PR per component, opportunistic | `refactor(components):` |
| 8 | C2 (React Compiler) | experiment branch, decide from data | — |

Tracks A and B have no interdependencies and can run in parallel. C1 benefits from landing
after the test-coverage campaign adds coverage to the chat components (safer extraction).

## Per-PR validation checklist

- `bun run type:check` and `bun run biome:check` green
- `bun run test` — no new failures, no newly skipped tests
- For B4/B6/C2: `bun run build` and compare chunk sizes; browser smoke test against the
  dev server (dashboard, task page, files tab, settings)
- For anything touching the SSE provider or task caches: run
  `src/features/events/contexts/__tests__/global-event-stream-provider.unit.test.tsx`
