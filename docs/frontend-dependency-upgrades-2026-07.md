# Frontend Dependency Upgrades & New-Feature Alignment — July 2026

Companion to the [frontend improvement plan](./frontend-improvement-plan-2026-07.md) and the
[frontend audit](./frontend-audit-2026-07.md). The improvement plan deferred *"Dependabot
vulnerability remediation"* as out of scope; this document is the deliberate, researched
version of that work — not just "bump to latest", but **which upgrades unlock new features or
best practices we should align to**, and which are risky enough to schedule carefully.

Two of the findings below (§0) are **latent correctness bugs**, not upgrades — version drift
that is already installed. Those come first.

### Method & verification

- **Installed vs. latest versions** were read from `bun outdated` in both the root and
  `apps/frontend` workspaces on 2026-07-11 — these version numbers are registry-authoritative.
- **New-feature / best-practice claims** came from web research against official
  blogs/changelogs/migration guides (linked inline). Treat feature descriptions and release
  dates as *research to re-confirm at implementation time*; the version numbers are solid, the
  narrative around them should be spot-checked against the linked changelog before you rely on
  a specific API.
- **Current usage** was verified by grep against `apps/frontend/src` and `.storybook`.

Paths are relative to `apps/frontend/` unless noted. Effort tags: trivial / small / medium / large.

---

## §0 — Version-drift bugs (fix regardless of the upgrade roadmap)

These are not "we could upgrade"; they are already-broken pins where companion packages have
crossed a major boundary the core hasn't.

### 0.1 Vitest core (v3) vs. `@vitest/*` companions (v4) — **P0, small–medium**

Installed today:

| Package | Installed | Notes |
|---|---|---|
| `vitest` | **3.2.4** | core |
| `@vitest/coverage-v8` | **4.0.8** | v4 |
| `@vitest/ui` | **4.1.9** | v4 |
| `@vitest/browser-playwright` | **4.0.8** | v4 |

The `@vitest/*` companions share internal APIs with the core and declare a same-major peer
(`vitest@^4`). Running them against `vitest@3` is unsupported and manifests as broken coverage
remapping, a UI that talks a changed RPC protocol, and a browser provider whose config shape
(`provider: playwright({...})` object) the v3 runner does not understand.

This is **not academic here**: `.storybook/main.ts` enables `@storybook/addon-vitest`, and
Storybook 10's addon runs stories as tests through Vitest **browser mode** with the Playwright
provider — exactly the path `@vitest/browser-playwright` sits on. So the drift touches the
Storybook test integration, not just a stray dev dependency.

**Fix:** move the whole Vitest stack to **v4.1.10** (latest) together. That is also the
strategic direction (§D4), so do it once. Requires the Vitest 4 migration in §D4.

> Note: `vitest.config.ts` already uses the v4-style `projects: [...]` array (introduced in
> 3.2) alongside `poolOptions.forks.singleFork`. The v4 migration renames the pool option to
> `maxWorkers: 1` + `isolate: false` — a small, mechanical edit.

### 0.2 `@storybook/test@8` alongside Storybook 10 — **P0, small**

`apps/frontend/package.json` pins `@storybook/test@^8.6.14` while the rest of Storybook is on
**10.0.6**. `@storybook/test` was deprecated in Storybook 9 and folded into the core
`storybook/test` sub-path export; keeping the v8 package next to SB10 is a cross-major
mismatch that breaks story `play`/interaction utilities at runtime.

Current usage is a single import:
`src/lib/components/ui/__stories__/sidebar-dialog.stories.tsx:2` — `import { fn } from '@storybook/test'`.

**Fix (mechanical):**
1. Remove `@storybook/test` from `apps/frontend/package.json`.
2. Change the import to `import { fn } from 'storybook/test'`.
3. Confirm `@storybook/addon-vitest` (already present) is the intended component-test runner.

- Refs: [Storybook 9 migration](https://storybook.js.org/docs/releases/migration-guide-from-older-version),
  [Storybook 10 migration](https://storybook.js.org/docs/releases/migration-guide)

**Acceptance for §0:** one Vitest major across `vitest` + all `@vitest/*`; no `@storybook/test`
package installed; `bun run test` and the Storybook Vitest project both run green.

---

## Track D — Dependency upgrades that unlock best practices

Ordered by leverage (feature/best-practice payoff × how directly it feeds the existing plan),
not by version-number size.

### D1. Build stack: Vite 8 + plugin-react 6 + React Compiler 1.0 — **medium; feeds plan C2**

| Package | Installed | Latest |
|---|---|---|
| `vite` | 7.2.2 | **8.1.4** |
| `@vitejs/plugin-react` | 4.7.0 | **6.0.3** |
| `babel-plugin-react-compiler` | *(not installed)* | latest |

The improvement plan's **C2 (Evaluate React Compiler)** is the anchor here, and the ecosystem
moved: the React Compiler reached **1.0 / stable** (per React's team) — it is no longer an
experiment gated behind "measure first for safety of the tool", only "measure first for
whether it helps *us*". The install story also changed with `@vitejs/plugin-react` v6, which
drops bundled Babel:

- **Lowest-effort path (stay on current plugin-react v4):** add
  `babel-plugin-react-compiler` and enable via `react({ babel: { plugins: ['babel-plugin-react-compiler'] } })`.
  This is the recommended way to run the C2 experiment **without** first doing the Vite 8 jump.
- **plugin-react v6 path (post Vite 8):** v6 removes Babel entirely; running any Babel plugin
  (including the compiler) needs `@rolldown/plugin-babel`, with the compiler wired via the
  `reactCompilerPreset()` helper the plugin now exports (plugin order matters — babel before react).

**Vite 7→8** is the larger piece: Rolldown becomes the default bundler and several config keys
rename. Directly relevant to *our* `vite.config.ts`:
- `build.rollupOptions` → `build.rolldownOptions` (we set `build.rollupOptions.output.manualChunks`).
- `optimizeDeps.esbuildOptions` → `optimizeDeps.rolldownOptions`; top-level `esbuild` → `oxc`.
- CSS minification switches to Lightning CSS; CJS default-import interop changes.
- Raised browser-target baseline and Node ≥ 20.19 / 22.12.

**Recommendation:** decouple the two. Run **C2 on the current v4 stack first** (cheap, reversible,
answers "does the compiler cut re-renders on our SSE-driven pages"). Treat **Vite 7→8 +
plugin-react 4→6** as its own branch with a build-output/chunk diff, because it rewrites the
bundler and our `manualChunks` config.

Also adopt **`eslint-plugin-react-hooks` v6** (flat config; ships the compiler-powered rules) —
this is the lint half of the compiler story and is useful even if C2 is not adopted.

- Refs: [React Compiler 1.0](https://react.dev/blog/2025/10/07/react-compiler-1),
  [React Compiler installation](https://react.dev/learn/react-compiler/installation),
  [Vite 8 announcement](https://vite.dev/blog/announcing-vite8),
  [Vite migration guide](https://vite.dev/guide/migration)

### D2. React 19.2 feature adoption — **trivial–small each; feeds plan B1/B5**

We're on `react@19.2.0` (latest 19.2.7) but use almost none of the 19.2 surface. Several map
directly onto plan items:

- **`useEffectEvent` (stable)** — removes the ref-as-escape-hatch pattern for effect callbacks
  that must read current state without re-subscribing. Ideal for the SSE subscription hooks in
  the event-stream provider and anywhere an effect closes over "current filter/user". *Feeds
  B5's "don't sync state in effects" cleanups.*
- **`useDeferredValue`** — the improvement plan's **B5** already proposes this for
  `canvas-search.tsx` debounce; confirmed as the native React 19 replacement for the
  effect-chained-state debounce.
- **`<Activity mode="hidden|visible">` (stable)** — preserves state but unmounts effects and
  defers updates for hidden subtrees. A better tool than `display:none` for the task page's
  collapsible chat / non-active tab panels, so hidden panels stop holding live subscriptions.
  *Complements the God-component decomposition in C1.*
- **`<Context>` as provider (stable)** — the improvement plan's **B2** codemod
  (`.Provider` → `<Context value>`) is confirmed as the React 19 direction; `.Provider` is
  deprecated. No new dependency needed — just do B2.
- **Refs as props** — `forwardRef` no longer required for new function components; relevant
  to C1 extractions and to the shadcn `forwardRef` removal (see D5).
- **React Performance DevTools tracks (19.2)** — free profiling for the C2 measurement and for
  the "re-renders on every SSE event" metric the plan calls out.

Keep `react`/`react-dom`/`@types/react*` on the latest 19.2.x patch.

- Ref: [React 19.2](https://react.dev/blog/2025/10/01/react-19-2)

### D3. TanStack Query & Router — **trivial upgrade; feeds plan A4 & B6**

| Package | Installed | Latest |
|---|---|---|
| `@tanstack/react-query` | 5.90.2 | **5.101.2** |
| `@tanstack/react-router` | 1.132.47 | **1.170.17** |
| `@tanstack/router-plugin` | 1.132.51 | **1.168.19** |

No breaking majors — still v5 / v1 — but ~38 router minors behind. The upgrade is safe and it
sharpens two existing plan items:

- **A4 (`queryOptions()` adoption):** the recommended payoff for *our* codebase is that
  `queryClient.setQueryData(opts.queryKey, …)` / `getQueryData(opts.queryKey)` become **typed**
  — exactly the imperative cache-patch sites in the SSE provider (`applyTaskEventUpdate`) and
  route loaders that A4 targets. The canonical pattern is `queryOptions()` factories consumed
  by `ensureQueryData` in loaders + `useSuspenseQuery` in components. Adopt as A4 describes.
- **B6 (route code-splitting):** rather than hand-authoring `.lazy.tsx` files, the current
  router-plugin supports **`autoCodeSplitting: true`** in the Vite plugin config — turn it on
  and cold routes split automatically. Prefer this over the manual `createLazyFileRoute`
  approach B6 sketches (still valid as a fallback for surgical control).
- Minor niceties worth knowing: router `defaultStaleReloadMode: 'background'` for snappier
  back-navigation; Query's `subscribed: false` for passive cache reads in the SSE provider.

- Refs: [queryOptions](https://tanstack.com/query/latest/docs/framework/react/guides/query-options),
  [Router + Query integration](https://tanstack.com/router/latest/docs/integrations/query)

### D4. Testing stack — **medium; resolves §0.1 and modernizes**

| Package | Installed | Latest |
|---|---|---|
| `vitest` + `@vitest/*` | 3.2.4 / 4.x | **4.1.10** (align all) |
| `@playwright/test` / `playwright` | 1.56.1 | **1.61.1** |
| `storybook` + addons | 10.0.6 | **10.5.0** |
| `jsdom` | 29.1.1 | current |

- **Vitest 3 → 4** (also fixes §0.1). Mostly mechanical: `poolOptions.forks.singleFork` →
  `maxWorkers: 1` + `isolate: false`; browser provider becomes an imported object
  (`provider: playwright({...})`); coverage `all`/`extensions` removed and reports shift
  (intentionally more accurate — re-baseline thresholds); a few spy/mock snapshot updates.
  Vitest 4 requires Vite ≥ 6 (we're on 7 — fine) and Node ≥ 20.
- **Storybook 10.0.6 → 10.5.0** + drop `@storybook/test` (§0.2); confirm `@storybook/addon-vitest`
  as the component-test runner (replaces the old test-runner).
- **Playwright 1.56 → 1.61** — additive: `page.localStorage`/`sessionStorage` APIs, WebAuthn
  virtual authenticator, `expect.soft.poll()`. One thing to check: 1.57 swapped Chromium for
  "Chrome for Testing", which can affect CI Docker images that reference the `chromium` binary
  path directly. Remove any `_react`/`_vue` experimental selectors (removed in 1.58).
- `jsdom` is current — no action.

- Refs: [Vitest 4](https://vitest.dev/blog/vitest-4), [Vitest migration](https://vitest.dev/guide/migration.html),
  [Playwright release notes](https://playwright.dev/docs/release-notes)

### D5. Styling: Tailwind 4.3, unified Radix, tailwind-merge, lucide — **trivial upgrades; feeds plan C3**

| Package | Installed | Latest |
|---|---|---|
| `tailwindcss` / `@tailwindcss/vite` | 4.1.14 | **4.3.2** |
| `tailwind-merge` | 3.3.1 | **3.6.0** |
| `lucide-react` | 1.17.0 | **1.24.0** |
| ~20× `@radix-ui/react-*` | mixed | → unified **`radix-ui`** |

- **C3 (semantic color tokens)** gets a concrete, current pattern. Tailwind v4's recommended
  approach for dark-mode-aware semantic tokens is to define raw values as CSS custom properties
  under `:root` / `.dark`, then register them with **`@theme inline`** so utilities embed the
  resolved reference (avoids a broken double-`var()`):

  ```css
  :root { --success-raw: oklch(0.72 0.11 155); /* … */ }
  .dark { --success-raw: oklch(0.82 0.13 155); /* … */ }
  @theme inline {
    --color-success: var(--success-raw);
    --color-warning: var(--warning-raw);
    --color-info:    var(--info-raw);
  }
  ```

  This generates `bg-success` / `text-success` / `border-success`, giving C3 its migration
  target for the 41 hardcoded palette classes. **Bump `tailwind-merge` to 3.6.0 in the same
  change** so `twMerge` understands v4.2/4.3 utilities (scrollbar, zoom, tab, `@container-size`).
- **Radix unified package:** the ~20 individual `@radix-ui/react-*` deps can collapse to a
  single tree-shakable `radix-ui` import, eliminating shared-internal version drift
  (`react-remove-scroll`, `aria-hidden`). shadcn ships an automated migration
  (`shadcn migrate radix`) and its new-york style now imports from `radix-ui`. This also tidies
  the `vendor-radix` `manualChunks` entry in `vite.config.ts`. *Small, automatable; do it
  alongside a shadcn CLI refresh.*
- **lucide-react 1.17 → 1.24** — additive icons/fixes. Keep named imports (`import { X } from
  'lucide-react'`) which tree-shake correctly under Vite; the `vendor-icons` chunk stays.
- Refs: [Tailwind v4.3](https://tailwindcss.com/blog/tailwindcss-v4-3),
  [Tailwind theme](https://tailwindcss.com/docs/theme),
  [Radix releases / unified package](https://www.radix-ui.com/primitives/docs/overview/releases),
  [shadcn radix migration](https://ui.shadcn.com/docs/changelog/2026-02-radix-ui)

### D6. Validation & AI/data libs — **zod trivial; ai major**

| Package | Installed | Latest | Effort |
|---|---|---|---|
| `zod` | 4.1.12 | **4.4.3** | small |
| `ai` (Vercel AI SDK) | 6.0.208 | **7.0.18** (latest 6.x = 6.0.221) | medium–large |

- **Zod 4.1 → 4.4** — same major, additive: `.exactOptional()`, `z.xor()`, `.slugify()`,
  experimental `z.fromJSONSchema()`, memory/perf improvements. Low-risk; adopt the new helpers
  opportunistically. (For new schemas, evaluate `z.interface()` for cleaner optional/recursive
  semantics.) A staleTime-free win to just move to 4.4.3.
- **Vercel AI SDK 6 → 7** — real major. **Node 22+ and ESM-only**, plus many renames
  (`system` → `instructions`, `onFinish` → `onEnd`, `fullStream` → `stream`, many
  `experimental_*` graduated). Official codemods cover most renames. Given our usage is
  concentrated (`src/lib/components/ai-elements/response.tsx` and the chat/streaming path),
  scope this as its own branch: run codemods → fix `system`/step-accumulation behavior →
  verify streaming render. **Staying on 6.0.221 (latest v6) is a fine interim** if we don't
  need v7 features yet.
- Refs: [Zod v4 changelog](https://zod.dev/v4/changelog),
  [AI SDK 7 migration](https://ai-sdk.dev/docs/migration-guides/migration-guide-7-0)

### D7. Tooling — **mostly trivial; TypeScript 7 is the judgment call**

| Package | Installed | Latest | Effort | Notes |
|---|---|---|---|---|
| `@biomejs/biome` | 2.1.1 | **2.5.3** | trivial | ~13% faster; 500+ rules; cross-file CSS/JSX unused-class detection; `--reporter=concise` (good for CI/agents); `recommended` → `preset` config rename. |
| `turbo` | 2.6.0 | **2.10.2** | trivial | Faster "time to first task", `turbo query` stable, git-worktree shared cache (helps parallel agents). |
| `esbuild` | 0.25.12 | **0.28.1** | trivial | `es2025` target, CSS `@scope`, text imports. No breaking changes for our usage. |
| `knip` | 6.17.1 | **6.23.0** | trivial | Plugin additions + bugfixes only. |
| `@commitlint/config-conventional` | 19.8.1 | **21.2.0** | trivial | Node 22+; output-format change only bites stdout parsers; our type/scope rules unchanged. |
| `typescript` | 5.9.3 | **7.0.2** | medium (staged) | **Do not blind-bump.** |

**TypeScript 7 (native/Go compiler)** is the one to stage carefully. The big win is build/
type-check speed (multiples faster). The catch in mid-2026 is the programmatic-API gap:
tooling that embeds the TS API — **`typescript-eslint`**, framework language services — is not
compatible until a later 7.x. Recommended staged approach:

1. Adopt `typescript@7` for the **`tsc --noEmit` CI/type-check job** only (fast, low-risk).
2. Keep a TS 6.x/5.x peer for `typescript-eslint` and editor tooling until the programmatic API
   lands.
3. Make `tsconfig.json` explicit about `strict`, `types`, `rootDir` before switching (7.0 changes
   several defaults).

Verify the programmatic-API/eslint status against the TS release notes **before** scheduling —
this is the fastest-moving item in this doc.

- Refs: [Biome 2.5](https://biomejs.dev/blog/biome-v2-5/),
  [TypeScript 7 announcement](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/)

### D8. Remaining majors with real migration cost — **schedule individually**

| Package | Installed | Latest | Effort | Blocker / caveat |
|---|---|---|---|---|
| `react-resizable-panels` | 3.0.6 | **4.12.1** | small–medium | **Blocked on shadcn.** We consume it via the shadcn wrapper `src/lib/components/ui/resizable.tsx`. v4 renames the public API (`PanelGroup`→`Group`, `PanelResizeHandle`→`Separator`, `direction`→`orientation`) and removes `onCollapse`/`onExpand`. Wait for the shadcn `<Resizable>` update or patch the wrapper locally. **Defer.** |
| `streamdown` | 1.4.0 | **2.5.0** | trivial–small | v2 is plugin-based — syntax highlighting / mermaid / math move to separate `@streamdown/*` packages. Used in `ai-elements/response.tsx`; add explicit plugin imports for whatever features that path relies on. Coordinate with the AI SDK v7 work (D6). |
| `js-yaml` | 4.1.1 | **5.2.1** | medium | Declared in `apps/frontend` but actually used in **`packages/figma`** (`yaml-generator.ts`, tests). v5 = named exports only, YAML 1.2 core schema by default (octal/merge-key/timestamp behavior changes), `Type` class replaced by `defineScalarTag` helpers, `load('')` throws. Migrate in the figma package and re-run its tests; consider dropping the unused frontend declaration. |
| `@figma/rest-api-spec` | 0.34.0 | **0.41.0** | small | Type-spec bump for the Figma feature; verify against `packages/figma` consumers. |
| `@pierre/diffs` / `@pierre/trees` | 1.1.21 / beta.3 | 1.2.12 / beta.5 | small | Diff-viewer deps; changelog check + visual diff smoke test. |
| `concurrently` | 9.2.1 | 10.0.3 | trivial | Dev-script runner; v10 major — check root scripts still spawn correctly. |

---

## Consolidated roadmap

| # | Work | Effort | Priority | Feeds plan item |
|---|---|---|---|---|
| 0 | §0.1 Vitest v4 alignment + §0.2 drop `@storybook/test` | small–med | **P0 (bug)** | — |
| 1 | D3 TanStack Query/Router bump + `autoCodeSplitting` | trivial | High | A4, B6 |
| 2 | D5 Tailwind 4.3 + tailwind-merge 3.6 + semantic `@theme inline` tokens | trivial–small | High | C3 |
| 3 | D2 React 19.2 features (`useEffectEvent`, `useDeferredValue`, `<Context>`, `<Activity>`) | trivial–small | High | B1, B2, B5, C1 |
| 4 | D7 Biome 2.5 / turbo 2.10 / esbuild / knip / commitlint bumps | trivial | Medium | — |
| 5 | D6 Zod 4.4 bump | small | Medium | A (schema types) |
| 6 | D5 Radix → unified `radix-ui` via shadcn migration | small | Medium | C1 (chunking) |
| 7 | D1 React Compiler 1.0 experiment on current v4 stack | small | Medium | **C2** |
| 8 | D1 Vite 7→8 + plugin-react 4→6 (own branch, chunk diff) | medium | Medium | C2, B6 |
| 9 | D4 Playwright 1.61 + Storybook 10.5 bumps | small | Medium | — |
| 10 | D6 AI SDK 6→7 (own branch, codemods) | medium–large | Low–Med | — |
| 11 | D7 TypeScript 7 staged (CI type-check first) | medium | Low (verify first) | — |
| 12 | D8 streamdown 2 / js-yaml 5 / figma specs / pierre / concurrently | mixed | Low | — |
| — | D8 react-resizable-panels 4 | small–med | **Deferred** | blocked on shadcn wrapper |

**Sequencing notes:** items 0–5 are safe, high-leverage, and independent — batch them into a
few small PRs first. Items 7–8 (compiler + Vite 8) share a theme but must stay decoupled: run
the compiler experiment on the current stack *before* the Vite 8 bundler migration so the two
sets of "did re-render counts change" and "did the bundle change" signals don't contaminate
each other. Items 10–11 are majors with hard Node/ESM/tooling constraints — gate them on their
own verification.

## Per-PR validation checklist (in addition to the improvement-plan checklist)

- `bun install` clean; `bun outdated` shows the intended packages moved and **no new major
  drift** between a core and its companions (the §0 failure mode).
- `bun run type:check`, `bun run biome:check`, `bun run test` green; re-baseline coverage
  thresholds after the Vitest 4 move (reports shift intentionally).
- For D1/D8 (bundler/build-affecting): `bun run build` and diff chunk output
  (`rollup-plugin-visualizer` is already a dev dep) + dev-server smoke test.
- For D6 AI SDK / D8 streamdown: exercise the chat streaming path (`ai-elements/response.tsx`).
- For anything touching the SSE provider or query cache: run
  `src/features/events/contexts/__tests__/global-event-stream-provider.unit.test.tsx`.
