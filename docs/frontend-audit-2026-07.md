# Frontend Codebase Audit — July 2026

Multi-agent audit of `apps/frontend/src` (~432 TS/TSX files) against the project's own
skill guidelines (`.claude/skills/frontend-dev-guidelines`) and current industry best
practices researched from primary sources (react.dev, TanStack docs, TkDodo, Tailwind v4
docs, OWASP, WCAG). Every finding of severity medium+ was adversarially verified by an
independent agent that read the cited code and attempted to refute the claim; verifier
corrections (mostly severity adjustments) are included inline. 62 findings confirmed,
8 refuted.

## Scorecard

| Dimension | Score /10 | Confirmed findings |
|---|---|---|
| testing | 3 | 8 |
| a11y-security | 3 | 6 |
| routing | 4 | 7 |
| performance | 5 | 5 |
| error-handling | 5 | 6 |
| components | 6 | 6 |
| state | 6 | 7 |
| styling | 6 | 7 |
| typescript | 6 | 6 |
| data-fetching | 7 | 4 |

---

## testing — score 3/10

The frontend test suite has 29 test files covering 402 source files, giving a roughly 7% file-level coverage ratio that is far below the project's own 70-80% overall goal. The tests that exist are generally well-structured (tier labels, AAA pattern, jest-dom matchers), but the suite has several systemic anti-patterns: widespread use of `act()` in hook tests instead of RTL's `waitFor`/`renderHook` update cycle, `fireEvent` used for click interactions instead of `userEvent`, `global.fetch` stubbed directly instead of MSW, entire feature areas (settings, repositories, figma, files, commands, agents, task-connections, etc.) with zero test coverage, and 4 core SSE invalidation tests permanently skipped leaving a critical real-time feature untested. The linter (Biome) has no testing-library-specific rules, so none of these patterns are caught at author time.

### Strengths
- jest-dom matchers are imported via setup.ts and used consistently throughout — `toBeInTheDocument()`, `toBeDisabled()`, `toHaveTextContent()` everywhere instead of raw property assertions.
- userEvent.setup() is used correctly with per-test instances in the external-source-message and error-boundary tests, and the majority of async `userEvent` calls are properly awaited.
- The custom `render` wrapper in test-utils/render.tsx correctly provides QueryClient, avoids production provider leakage, and exports a re-usable `createTestQueryClient` with retry:false.
- Tier labeling (`Tier 1`, `Tier 2`) appears in describe block names and file names (`.unit.test.tsx`), giving the suite good discoverability structure.
- The `queryBy*` variants are used only for absence assertions (`expect(screen.queryBy...).not.toBeInTheDocument()`), which is the correct pattern throughout.
- The `vitest.config.ts` correctly configures `jsdom` environment, a global setup file, and separate test patterns for unit/integration/e2e allowing tiered execution per CLAUDE.md.

### Confirmed findings

#### [critical] Massive coverage gap — 15 of 19 feature modules have zero tests

**Files:** `apps/frontend/src/features/settings/ (31 source files, 0 tests)`, `apps/frontend/src/features/repositories/ (15 source files, 0 tests)`, `apps/frontend/src/features/figma/ (14 source files, 0 tests)`, `apps/frontend/src/features/files/ (13 source files, 0 tests)`, `apps/frontend/src/features/commands/ (8 source files, 0 tests)`, `apps/frontend/src/features/agents/ (5 source files, 0 tests)`, `apps/frontend/src/features/task-connections/ (5 source files, 0 tests)`, `apps/frontend/src/features/task-templates/ (5 source files, 0 tests)`, `apps/frontend/src/routes/ (28 route files, 0 tests)`

Out of 402 source files and 19 feature directories, 15 features have exactly zero test files. Settings (31 source files covering Claude, GitHub, GitLab, Figma, API-key configurations), repositories (15 files), and all 28 route files are completely uncovered. The project's own guidelines specify 70-80% overall coverage and 80-90% for Tier-1 critical paths. Settings are Tier 1 by any measure — connection forms that misconfigure API credentials are a user-facing data-loss risk.

**Recommendation:** Start with the three highest-ROI targets: (1) `features/settings` — write integration-layer tests for the connection form dialogs using the custom render wrapper and mocked API hooks; (2) `features/repositories` — test the branch selection and repository validation flows; (3) route files — add at minimum a smoke test per route verifying loading state, error boundary, and basic render. Aim for 2-3 focused tests per Tier-2 component, not exhaustive coverage.

> **Verifier correction:** The auditor undercounts both the total feature directories (22, not 19) and the zero-test directories (16, not 15). The project has 22 feature directories total; 16 have no tests and 6 have partial coverage. The CLAUDE.md guidelines state ">80% coverage on critical paths (Tier 1)" — not a "70-80% overall" target. These are minor inaccuracies that do not change the overall severity: the coverage gap is real, large, and the settings/repositories/routes gaps are confirmed. The claimed severity (critical) is appropriate given the breadth of uncovered critical-path code.

#### [high] Four critical SSE invalidation tests permanently skipped — key feature untested

**Files:** `apps/frontend/src/features/events/contexts/__tests__/global-event-stream-provider.unit.test.tsx:100`, `apps/frontend/src/features/events/contexts/__tests__/global-event-stream-provider.unit.test.tsx:143`, `apps/frontend/src/features/events/contexts/__tests__/global-event-stream-provider.unit.test.tsx:197`, `apps/frontend/src/features/events/contexts/__tests__/global-event-stream-provider.unit.test.tsx:236`

The `GlobalEventStreamProvider` is the central real-time event bus — every task status change, sandbox event, and repository update flows through it. The four tests covering per-task query invalidation, cross-task isolation, legacy subType handling, and sandbox-specific invalidation are all marked `it.skip`. They were written but never made to pass. The file also directly mocks `global.fetch` with a hand-rolled streaming stub (line 75), which does not actually exercise the streaming read loop at all.

**Recommendation:** Remove the `it.skip` markers and fix the tests. The root cause is that the tests try to spy on `global.EventSource` (line 107) but the component uses `fetch` with a ReadableStream, not EventSource. Replace the hand-rolled fetch stub with a proper ReadableStream mock that delivers encoded SSE lines and drives the real read loop. Alternatively, extract the event-dispatch logic into a pure function and test that in isolation.

> **Verifier correction:** The finding is real but the severity is inflated. The four `it.skip` tests exist as claimed and are structurally broken (they check for `global.EventSource` which the implementation never uses — it uses `fetch` instead, confirmed by reading the provider source). However, the invalidation logic they aim to cover is actually tested by the companion `event-handlers.unit.test.ts` file, which has multiple passing, non-skipped tests covering per-task query key invalidation, sandbox-specific invalidation, absence of broad `tasks.all` invalidation, and legacy subType handling. The real issue is test debt from an `EventSource`→`fetch` migration that was never cleaned up, not a completely untested feature. Severity should be **medium** (stale/broken tests that should be removed or rewritten) rather than high (critical untested behavior).

#### [high] Pervasive explicit `act()` wrapping in hook tests instead of RTL-managed async updates

**Files:** `apps/frontend/src/lib/components/chat/file-autocomplete/__tests__/use-file-autocomplete.unit.test.ts:38,50,56,67,77,90,109,114,130,140,151,170,178,195,199,216,233,237,254,259,263,268,286,313,318,334,339,357,361,379,383,401,405,430,437,452,457,465,480,485,503,510,528,533,539,558,564,580,587,608,625,631,686,691`, `apps/frontend/src/features/tasks/contexts/__tests__/task-ui-provider.unit.test.tsx:90,99`, `apps/frontend/src/features/tasks/contexts/__tests__/derived-state-patterns.unit.test.tsx:92,203`, `apps/frontend/src/lib/components/ui/__tests__/mobile-sidebar-drawer.test.tsx:17`, `apps/frontend/src/features/tasks/hooks/__tests__/use-task-draft-message.test.ts:28,35,57,60`

The `use-file-autocomplete` test file contains approximately 55 explicit `act()` calls — nearly every state-mutating call is wrapped manually. RTL's `renderHook` already wraps `act()` around state updates; explicit wrapping is redundant and is flagged by the RTL docs as a code smell indicating either incorrect async handling or unnecessary ceremony. In `task-ui-provider.unit.test.tsx` lines 90 and 99, `.click()` is called directly on the DOM node inside `act()` instead of using `userEvent`, bypassing pointer events and focus.

**Recommendation:** For synchronous hook state updates tested with `renderHook`, remove the `act()` wrappers — RTL handles them internally. For async updates, use `await waitFor(() => expect(...))`. Replace `act(() => { screen.getByTestId('switch-to-task-2').click() })` with `const user = userEvent.setup(); await user.click(screen.getByRole('button', { name: 'Task 2' }))` to dispatch realistic event cascades.

> **Verifier correction:** The finding is real but severity should be medium, not high. The pattern exists as described: use-file-autocomplete.unit.test.ts has ~55 explicit act() wraps around synchronous calls (handleInputChange, selectFile, handleKeyDown), task-ui-provider.unit.test.tsx uses .click() on DOM nodes inside act() at lines 90 and 99, derived-state-patterns.unit.test.tsx wraps rerender() in act() at line 203, and use-task-draft-message.test.ts wraps setState setter calls in act() at lines 28, 35, 57, 60. However, two mitigating factors: (1) the async act() usages in the debounced-search tests (await act(async () => { await new Promise(...) })) are correct and necessary; (2) the mobile-sidebar-drawer.test.tsx act() at line 17 is legitimate — raw DOM events dispatched on document outside React's synthetic system do require explicit act() to flush state. The issue is a genuine code smell causing redundant ceremony, but it does not cause test failures or mask bugs, making high severity an overstatement.

#### [high] `fireEvent.click` and `fireEvent.change` used for user interactions instead of `userEvent`

**Files:** `apps/frontend/src/features/tasks/components/chat-view/__tests__/command-buttons.unit.test.tsx:49,123,124,142`, `apps/frontend/src/features/tasks/components/task-header/__tests__/scheduled-jobs-pill.unit.test.tsx:30`, `apps/frontend/src/lib/components/chat/message-actions/__tests__/feedback-dialog.unit.test.tsx:57`

`fireEvent.click` fires a single synthetic click event, skipping the full pointer event chain (pointerdown → mousedown → focus → pointerup → mouseup → click). This means focus management, blur handlers, and pointer-based event guards are not exercised. In `command-buttons.unit.test.tsx` the disabled-button test at line 142 uses `fireEvent.click` — because fireEvent bypasses the button's native disabled check, this test can pass even if the disabled guard is broken. In `feedback-dialog.unit.test.tsx:57`, `fireEvent.change` is mixed with `userEvent.click` in the same test, creating an inconsistent simulation.

**Recommendation:** Replace all `fireEvent.click(element)` with `await user.click(element)` (where `user = userEvent.setup()` per test). Replace `fireEvent.change(textarea, { target: { value: '...' } })` with `await user.clear(textarea); await user.type(textarea, '...')`. This ensures the full event chain is exercised, including focus/blur and React's synthetic event layer.

> **Verifier correction:** The finding is real but severity should be reduced from high to low/medium. The specific claim that the disabled-button test at line 142 "can pass even if the disabled guard is broken" is incorrect — React 19's event delegation correctly suppresses onClick on native `&lt;button disabled&gt;` elements even when using fireEvent. The only genuinely impactful inconsistency is in feedback-dialog.unit.test.tsx where fireEvent.change is mixed with userEvent.click in the same test, meaning keystroke-level validation on the textarea is not exercised. The rest of the fireEvent.click usages (popover trigger in scheduled-jobs-pill, non-disabled click in command-buttons) are minor style deviations that do not create meaningful testing gaps.

#### [high] No MSW — `global.fetch` stubbed directly, mocking below the network boundary

**Files:** `apps/frontend/src/features/events/contexts/__tests__/global-event-stream-provider.unit.test.tsx:75`

MSW is not installed anywhere in the project (no `msw` package in package.json). The one test that needs to exercise network behavior stubs `global.fetch` directly with a hand-rolled object. This approach does not validate HTTP request headers, query parameters, status code handling, or streaming response parsing — bugs in those areas pass silently. The project's own guidelines say to mock external dependencies, but the recommended approach for HTTP is at the network boundary, not the fetch function.

**Recommendation:** Install MSW (`bun add -d msw`) and configure a shared server in `test-utils/setup.ts` with `server.listen({ onUnhandledRequest: 'error' })` in `beforeAll` and `server.resetHandlers()` in `afterEach`. Replace the `global.fetch = vi.fn()` stub with `http.get('/api/v1/events/stream', () => { ... })` handlers that return proper `HttpResponse` objects. This validates the real fetch call path.

> **Verifier correction:** The finding is real but the severity framing is off. The actual problem is that all four tests in this file are marked it.skip, meaning GlobalEventStreamProvider has zero active test coverage from this file — not that a bad stub is silently swallowing bug signals. The global.fetch stub exists and is poor methodology (no status, no real headers, no real ReadableStream), but it is currently dead code. The severity should be characterized as: (1) no active test coverage of this component at all (the skipped tests are the primary problem), and (2) when/if those tests are re-enabled, the fetch stub would be substandard. The issue is real but the framing should prioritize "tests are all skipped = zero coverage" over "stubbing below the network boundary."

#### [medium] Synchronous `userEvent` API used (no `userEvent.setup()`) across multiple test files

**Files:** `apps/frontend/src/lib/components/chat/chat-input/__tests__/chat-input.unit.test.tsx:23,34,45,56,67,116,140,155,166`, `apps/frontend/src/lib/components/chat/message-actions/__tests__/feedback-dialog.unit.test.tsx:27,60`, `apps/frontend/src/lib/components/chat/chat-input/selectors/__tests__/popover-selector.unit.test.tsx:134`, `apps/frontend/src/lib/components/chat/message-actions/__tests__/message-action-bar.unit.test.tsx:30`

These files call `await userEvent.click(el)` and `await userEvent.type(el, str)` without first calling `userEvent.setup()`. In userEvent v14, the recommended pattern is `const user = userEvent.setup()` once per test to create an isolated instance with its own pointer and clipboard state. Using the module-level static API creates a shared global state across tests in the same file, which can leak pointer positions or clipboard contents between tests and produce non-deterministic results in parallel runs.

**Recommendation:** In each `describe` block or `test`, add `const user = userEvent.setup()` before the first interaction, then call `user.click(el)` and `user.type(el, str)` through that instance. This is a straightforward search-and-replace: `await userEvent.click(x)` → `const user = userEvent.setup(); await user.click(x)`.

> **Verifier correction:** The pattern is real and present across all four cited files exactly as described. The severity should be low rather than medium: in practice, with Vitest's serial intra-file test execution and these tests using only click/type/keyboard/hover (not clipboard APIs), the shared global state rarely produces actual non-determinism. The fix (adding `const user = userEvent.setup()` per test and replacing module-level calls) is valid and represents an improvement to test hygiene, but the real-world risk of test failures is low.

#### [medium] `container.querySelector` used to select elements by CSS class, bypassing accessible queries

**Files:** `apps/frontend/src/lib/components/chat/message-history/__tests__/message-history.unit.test.tsx:302`, `apps/frontend/src/lib/components/chat/chat-input/__tests__/chat-input.unit.test.tsx:222`, `apps/frontend/src/lib/components/chat/chat-input/selectors/__tests__/popover-selector.unit.test.tsx:122`

`container.querySelector('.overflow-y-auto')` at message-history.unit.test.tsx:302 selects by an internal CSS class name. If the class is ever renamed or the component is refactored (e.g., switched to a CSS Module or Tailwind v4 variant), this test silently returns `null` and the test either skips its assertions (there is a null guard on line 304 that returns early) or fails with an unhelpful 'Cannot read properties of null' error. The test at chat-input.unit.test.tsx:222 uses `container.querySelector('form')` which is slightly more stable but still bypasses RTL's accessible queries.

**Recommendation:** For the scroll container, add a `data-testid='scroll-container'` to the `MessageHistory` component's outer scrollable div (this is a legitimate use of testid for an element with no accessible anchor), then query it with `screen.getByTestId`. For the form, use `screen.getByRole('textbox').closest('form')` which is already done in another test in the same file — use that pattern consistently. For the SVG check in popover-selector, add an `aria-label` to the icon wrapper and use `getByRole('img', { name: /file/i })`.

> **Verifier correction:** The finding is real at medium severity, but only the CSS class selector at message-history.unit.test.tsx:302 is the clear violation. The `container.querySelector('form')` in chat-input.unit.test.tsx:222 and `container.querySelector('svg')` in popover-selector.unit.test.tsx:122 use HTML element tag selectors, which are semantic and more stable than class-based selectors — they should not be cited as equivalent issues. The scope should be narrowed to just the `.overflow-y-auto` case, and the fix is to add a `data-testid="scroll-container"` to the scroll container div in message-history.tsx and query by `screen.getByTestId('scroll-container')` in the test.

#### [medium] Redundant `cleanup()` call in global setup conflicts with RTL's automatic cleanup

**Files:** `apps/frontend/src/test-utils/setup.ts:146-149`

The global setup file calls `cleanup()` inside `afterEach`, which is correct but redundant: `@testing-library/react` v14+ automatically runs `cleanup()` after each test when configured with a test runner that supports it (Vitest does). Having an explicit `cleanup()` call alongside the automatic one is at minimum dead code, and can cause double-unmounting warnings in some React versions. More importantly, `vi.clearAllMocks()` is called in the same hook, which clears mocks set up in `beforeAll` or module-level `beforeEach`, potentially breaking tests that rely on persistent mock state across renders.

**Recommendation:** Remove the explicit `cleanup()` call from `afterEach` in `setup.ts`; rely on RTL's automatic cleanup. If specific tests need cleanup guarantees, call `cleanup()` in their own `afterEach`. Keep `vi.clearAllMocks()` but document that it runs globally — some tests may need `vi.resetAllMocks()` instead, which also resets return values.

> **Verifier correction:** The redundant cleanup() call is real but low severity (dead code, not harmful). The vi.clearAllMocks() risk described by the auditor is largely inaccurate: module-level vi.mock() factory registrations are unaffected by clearAllMocks(), so the module mocks for mermaid, streamdown, react-resizable-panels, and radix-ui tooltips persist correctly across tests. The actual severity should be low (cosmetic/dead code), not medium. The cleanup() call can simply be removed since RTL v16 handles it automatically with Vitest globals.

---

## a11y-security — score 3/10

The frontend has one critical XSS vulnerability in the `streamdown` library used for all LLM/agent-generated markdown: `rehype-raw` is enabled in the default plugin set, which passes raw HTML from LLM output through to the DOM, while `rehype-harden` only sanitizes `<a>` and `<img>` elements — it does not strip `<script>`, `<iframe>`, `on*` event handlers, or other dangerous tags. Combined with zero CSP headers on the Nginx layer, this creates a real stored-XSS risk in a product whose entire surface is agent-generated text. Accessibility gaps are pervasive but lower-severity: no skip-navigation link, no SPA focus management after route changes, several icon-only buttons missing accessible names, and no `@media (prefers-reduced-motion)` override for custom CSS transitions. The majority of dialogs use Radix UI which handles focus trapping correctly, and the codebase consistently applies `rel="noopener noreferrer"` on external links with only two minor deviations.

### Strengths
- Radix UI primitives (shadcn/ui Dialog, Popover, Select) are used throughout for interactive overlays, providing correct keyboard focus trapping and Escape-to-close behaviour with no custom trap needed.
- The vast majority of `target="_blank"` external links correctly include `rel="noopener noreferrer"` — only two minor deviations exist across 12 total instances.
- Mermaid is initialised with `securityLevel: 'strict'` inside `streamdown`, which limits what diagram syntax can be embedded, providing a partial mitigation for the SVG injection vector.
- No auth tokens or sensitive credentials are stored in `localStorage` — only benign UI state (theme, sidebar open state, task draft messages) uses local/session storage.
- The `MarkdownPreview` component used for file previews (`react-markdown` + `remark-gfm`) does NOT include `rehype-raw`, so raw HTML in markdown files is escaped rather than rendered, limiting that attack surface to file content only.
- Icon buttons in the lib/components and chat areas consistently use `aria-label` (e.g., remove queued message, toggle sidebar, message action bar). The preview header also correctly labels reload and inspector buttons.
- All dialogs link form controls to their `<label>` elements via `htmlFor`/`id` associations (verified in feedback, create-task, and settings forms), and the sidebar filter panel uses shadcn `<Label>` above each `<SelectTrigger>`.

### Confirmed findings

#### [critical] No Content-Security-Policy header deployed — XSS has no server-side fallback

**Files:** `docker/nginx.conf:1`

The Nginx configuration serving the frontend SPA contains no `Content-Security-Policy`, `X-Frame-Options`, `X-Content-Type-Options`, or `Referrer-Policy` response headers. Without a CSP, any successful XSS injection (see finding 1) can exfiltrate session cookies, call the API on the victim's behalf, or load external scripts with no browser-enforced restriction. `X-Frame-Options: DENY` is also absent, leaving the app open to clickjacking.

**Recommendation:** Add the following headers to the Nginx `server {}` block at minimum:
```nginx
add_header Content-Security-Policy "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https:; connect-src 'self'; frame-ancestors 'none';" always;
add_header X-Frame-Options DENY always;
add_header X-Content-Type-Options nosniff always;
add_header Referrer-Policy strict-origin-when-cross-origin always;
```
Note: `shiki` and `mermaid` use dynamic imports; adjust `script-src` after verifying no eval/blob URLs are needed. Consider `require-trusted-types-for 'script'` as a longer-term goal.

#### [high] No SPA focus management after route changes — screen reader users lose orientation

**Files:** `apps/frontend/src/lib/layout/index.tsx:11`, `apps/frontend/src/routes/__root.tsx:116`

TanStack Router performs client-side navigation without any programmatic focus movement. The `<Layout>` component renders a plain `<main>` element with no `tabIndex` or `ref`, and there is no `useEffect` watching `pathname` to move focus to the main content region after navigation. Screen reader and keyboard users have no signal that the page has changed — NVDA/JAWS continues reading from wherever focus was before the route change, and the virtual cursor does not move.

**Recommendation:** Add a `tabIndex={-1}` `ref` to the `<main>` element in `apps/frontend/src/lib/layout/index.tsx` and call `mainRef.current?.focus()` in the root component's `useEffect` whenever the pathname changes:
```tsx
// In __root.tsx or layout/index.tsx:
const mainRef = useRef<HTMLElement>(null)
const { pathname } = useRouterState({ select: s => s.location })
useEffect(() => { mainRef.current?.focus() }, [pathname])
// ...
<main ref={mainRef} tabIndex={-1} className="...">...</main>
```
Also add a visually-hidden skip-navigation link as the very first focusable element in the DOM pointing to `#main-content`.

> **Verifier correction:** The finding is real and accurately located. The severity claim of "high" is at the upper bound: the application is an authenticated task-management SPA rather than a public content site, and many interactions occur within a single persistent shell without full route transitions. A severity of "medium" or "medium-high" would better reflect the realistic impact, while still acknowledging the genuine WCAG AA gap. The recommended fix (add `tabIndex={-1}` and a `ref` to `<main>`, add a `useEffect` in the Layout or RootComponent keyed on the router's resolved location to call `mainRef.current?.focus()`, and add a skip-navigation link) is valid and would work correctly in this codebase.

#### [high] Multiple icon-only buttons lack accessible names

**Files:** `apps/frontend/src/features/navigation/components/top-bar.tsx:53`, `apps/frontend/src/features/preview/components/inspector-panel.tsx:86`, `apps/frontend/src/features/repositories/components/sections/servers-group.tsx:68`, `apps/frontend/src/features/sandbox/components/sandbox-overview.tsx:192`, `apps/frontend/src/features/sandbox/components/sandbox-diagnostics.tsx:486`

Several `<Button size="icon">` controls that render only a Lucide icon have neither an `aria-label` nor a `title` attribute, making them nameless interactive elements that screen reader users cannot identify. Confirmed missing names: (1) the main application menu/settings button in `TopBar` (renders `<Settings>` icon, opens a popover with all navigation), (2) the close button in `InspectorPanel` (renders `<X>` icon), (3) the server delete button in `ServersGroup` (renders `<Trash2>` icon), (4) the back-navigation button in `SandboxOverview` and `SandboxDiagnostics` (renders `<ArrowLeft>` icon inside a `<Link>`).

**Recommendation:** Add `aria-label` to each icon-only button:
```tsx
// top-bar.tsx:53
<Button variant="ghost" size="icon" aria-label="Open main menu">
  <Settings className="h-5 w-5" aria-hidden="true" />
</Button>

// inspector-panel.tsx:86
<Button variant="ghost" size="icon" onClick={onClose} aria-label="Close inspector">
  <X className="h-3 w-3" aria-hidden="true" />
</Button>

// servers-group.tsx:68
<Button variant="ghost" size="icon" aria-label="Remove server" onClick={...}>
  <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
</Button>
```
Also add `aria-hidden="true"` to all icon SVGs inside labelled buttons to prevent screen readers from announcing the icon name in addition to the button label.

#### [medium] Server-provided URLs rendered in href with no scheme validation (open-redirect / javascript: risk)

**Files:** `apps/frontend/src/features/task-connections/components/connection-chip.tsx:87`, `apps/frontend/src/features/tasks/components/task-header/task-info-section.tsx:192`, `apps/frontend/src/features/sandbox/components/sandbox-diagnostics.tsx:198`, `apps/frontend/src/features/preview/components/publish-button.tsx:28`

`connection.url`, `task.siteUrl`, and `server.url` are server-supplied strings that originate from the backend (and in turn from external integrations like GitHub, Slack, Linear, or from the sandbox agent). These values are placed directly into `href` attributes without any URL validation to block `javascript:` or `data:` schemes. If a malicious integration, webhook, or sandbox agent sets one of these URL fields to `javascript:alert(document.cookie)`, clicking the link in the UI will execute JavaScript in the page's origin context. `connection.url` specifically comes from third-party adapters (GitHub PR URLs, Linear issue URLs, Slack message URLs) where the backend may relay attacker-controlled content.

**Recommendation:** Validate all server-supplied URLs before rendering them into `href`. Add a shared utility:
```ts
// lib/utils/url.ts
export function isSafeUrl(url: string | null | undefined): boolean {
  if (!url) return false;
  try {
    const parsed = new URL(url);
    return parsed.protocol === 'https:' || parsed.protocol === 'http:';
  } catch { return false; }
}
```
Then gate `href` rendering:
```tsx
href={isSafeUrl(connection.url) ? connection.url : '#'}
```
Alternatively, use the URL object and check `parsed.protocol` before rendering. The backend should also validate and sanitize these URLs before storing them.

> **Verifier correction:** The finding is real and the severity (medium) is correct. One minor overstatement: the `rel="noreferrer"` complaint applies only to connection-chip.tsx; the other three files use the correct `rel="noopener noreferrer"`. The primary XSS risk is in `connection.url` (connection-chip.tsx), which relays third-party integration URLs. The `siteUrl` and `server.url` cases are genuine missing-validation issues but are lower real-world risk because those values are infrastructure-generated by the backend rather than relayed from arbitrary external parties.

#### [medium] Missing rel="noopener" on two external links and no prefers-reduced-motion override

**Files:** `apps/frontend/src/features/task-connections/components/connection-chip.tsx:89`, `apps/frontend/src/features/tasks/components/task-header/task-info-section.tsx:194`, `apps/frontend/src/lib/styles/globals.css:226`

Two `target="_blank"` links use only `rel="noreferrer"` but omit `rel="noopener"`. While `noreferrer` implies `noopener` in modern browsers (Chromium 88+, Firefox 79+), older browsers do not apply this implication and same-origin `_blank` links in Safari older than 12.1 remain vulnerable to reverse tabnabbing. The canonical recommendation is to always include both. Separately, the global CSS defines a `transition: background-color 0.2s` on the scrollbar thumb (line 226) and similar transitions throughout, with no `@media (prefers-reduced-motion: reduce)` override block anywhere in the stylesheet or JavaScript. Users with vestibular disorders who have enabled reduced motion will still see all transitions.

**Recommendation:** For the link issue: change both instances to `rel="noopener noreferrer"`. For reduced motion: add to `globals.css`:
```css
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
    scroll-behavior: auto !important;
  }
}
```
Also check the `animate-spin` class used in loading spinners — consider replacing spinner animations with `visibility: hidden` under reduced-motion.

> **Verifier correction:** The finding is real (both issues exist at the cited locations) but the severity should be lowered to "low" rather than "medium". The `noreferrer` claim overstates the security risk: `noreferrer` has implied `noopener` in all major browsers since 2019-2021 and the reverse tabnabbing vulnerability does not apply to users of this tool in practice. The prefers-reduced-motion gap is genuine but the specific transitions cited are color-only (not spatial motion), so the harm to vestibular disorder users is more limited than claimed. The recommendation to add `noopener` alongside `noreferrer` is valid as a best-practice hygiene fix, and adding a `@media (prefers-reduced-motion: reduce)` block disabling transitions is also valid, but both are low-priority improvements rather than medium-severity security/accessibility issues.

#### [medium] No skip-navigation link — keyboard users must tab through entire sidebar on every page

**Files:** `apps/frontend/src/lib/layout/index.tsx:11`, `apps/frontend/src/routes/__root.tsx:116`

No skip-navigation link exists anywhere in the HTML output. Keyboard users and switch-access users must tab through the entire persistent sidebar navigation (task list, filters, top bar) on every page load and route change before reaching the main content area. WCAG 2.4.1 (Bypass Blocks) requires a mechanism to bypass repeated navigation, and a visually-hidden skip link is the standard implementation for SPAs.

**Recommendation:** Add a skip link as the very first element inside `<body>` (or the root component):
```tsx
// In __root.tsx RootComponent, before all other content:
<a
  href="#main-content"
  className="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50 focus:px-4 focus:py-2 focus:bg-background focus:text-foreground focus:rounded focus:border"
>
  Skip to main content
</a>
// Then give the <main> element the matching id:
<main id="main-content" tabIndex={-1} className="...">
```
This is typically invisible until focused, so it has zero visual impact on sighted users.

> **Verifier correction:** The issue is real but narrower than described. The skip-navigation link gap primarily affects the /tasks/$task-id route, where AppSidebar renders a sizeable task list that keyboard users must tab through before reaching main content. The root Layout component and most other pages (dashboard at /, /tasks/, /settings, /sandbox, etc.) do not include the persistent sidebar — they render only a slim TopBar or no persistent navigation at all. The description "must tab through entire sidebar on every page" should read "must tab through the task-list sidebar on the task detail view (/tasks/$task-id)." The severity (medium) and the WCAG 2.4.1 citation remain accurate. The fix — a visually-hidden skip link plus an id on the main content container — should be added to the task detail route layout, not to the global Layout component.

### Refuted claims (checked and dismissed)

- **Stored XSS via raw HTML in LLM-generated markdown (streamdown + rehype-raw)** — The auditor's claim is based on a fundamental misunderstanding of how react-markdown + rehype-raw + hast-util-to-jsx-runtime work together in a React CSR application.

**What the auditor got right:**
- `rehype-raw` (Rt) is confirmed as a default plugin in streamdown's `_t` object at `/opt/moatless/workspace/moatless-vibe/node_modules/streamdown/dist/index.js`. It re-parses raw HTML nodes into HAST element nodes.
- `rehype-harden` only rewrites `<a href>` and `<img src>` URLs; all other element t

- **Mermaid SVG injected via dangerouslySetInnerHTML without sanitisation** — The claim is factually incorrect on the key technical point. While streamdown's Mermaid component (`oe`) does not call DOMPurify itself before passing SVG to `dangerouslySetInnerHTML`, the auditor missed that **Mermaid 11.x internally runs DOMPurify on the SVG output before returning it from `render()`**. Reading `/opt/moatless/workspace/moatless-vibe/node_modules/mermaid/dist/mermaid.core.mjs` confirms the exact sanitization path:

```
const isSandboxed = config.securityLevel === SECURITY_LVL_S

---

## routing — score 4/10

The routing layer uses TanStack Router with file-based routes and shows several genuine strengths: `defaultPreloadStaleTime: 0` is correctly set, `validateSearch` with Zod `.catch()` is used on most routes, and type-safe `<Link to="..." params={...}>` is used consistently throughout feature components. However, zero routes define a `loader`, contrary to the project's own skill guidelines that mandate prefetching via loaders. Auth is handled via a component gate rather than `beforeLoad` on a pathless layout route, there is no `errorComponent` on any route, there is no route-level code splitting (no `.lazy.tsx` files), and several navigation calls use string template literals or raw `as` casts that forfeit type safety.

### Strengths
- defaultPreloadStaleTime: 0 is set on the router, correctly deferring cache ownership to TanStack Query (main.tsx line 25).
- validateSearch with Zod schemas and .catch() fallbacks is used on all dashboard and task detail routes, preventing malformed URLs from throwing (index.tsx, dashboard.tsx, tasks/index.tsx, task-search-schema.ts).
- Type-safe <Link to='/tasks/$task-id' params={{ 'task-id': id }}> is used consistently in feature components (task-card.tsx:368, sidebar-tasks.tsx:231, trigger detail, connection list).
- navigate({ search: (prev) => ({ ...prev, ... }) }) with updater functions is used correctly in use-task-view.ts for all in-task search param mutations, preserving unrelated params.
- The admin route uses beforeLoad with queryClient.ensureQueryData for role-based access control, correctly blocking child loaders and components (admin.tsx lines 10-19).

### Confirmed findings

#### [high] No route loaders: all data fetched inside components after render

**Files:** `apps/frontend/src/routes/tasks/$task-id/index.tsx`, `apps/frontend/src/routes/admin/triggers/$trigger-id/index.tsx`, `apps/frontend/src/routes/admin/integrations/connections/$connection-id/index.tsx`, `apps/frontend/src/routes/admin/triggers/index.tsx`

Zero routes define a `loader` function. Every page fetches data inside the component using TanStack Query hooks (useQuery). This means the user sees a loading skeleton after navigation completes, not before. The project's own skill guideline (routing.md lines 194-210, 520-545) explicitly calls this an anti-pattern and mandates `queryClient.ensureQueryData` inside loaders so data is warm before the component renders. With `defaultPreload: 'intent'` already set, loaders on hover/focus links would preload data before the user even clicks.

**Recommendation:** Add a `loader` to each route that fetches primary data, e.g.: `loader: ({ params }) => queryClient.ensureQueryData({ queryKey: queryKeys.tasks.detail(params['task-id']), queryFn: () => taskApi.getTask(params['task-id']) })`. Start with the highest-traffic routes: `/tasks/$task-id/` and `/admin/triggers/$trigger-id/`.

> **Verifier correction:** The finding is real and correctly described. Minor severity adjustment: the three admin-only pages (triggers list, trigger detail, connection detail) are lower-traffic and "medium" severity is more accurate for those. The tasks/$task-id route is the primary user flow and warrants "high" severity. Also worth noting: the connection detail page fetches all connections and filters client-side rather than fetching a single connection by ID — an additional inefficiency beyond the loader gap. The recommendation to use `queryClient.ensureQueryData` inside `loader` functions is valid and would work in this codebase since `queryClient` is already imported as a module-level singleton in the existing `admin.tsx` `beforeLoad`.

#### [high] Auth guard is component-based, not beforeLoad on a pathless layout route

**Files:** `apps/frontend/src/routes/__root.tsx`, `apps/frontend/src/features/auth/components/auth-gate.tsx`

Authentication is implemented as a React component (`AuthGate`) rendered inside the root component's JSX, with a conditional check on `pathname`. This means:
1. The component renders before the redirect, so child route loaders and components can start (and fail) before the auth redirect fires.
2. The current URL is not preserved in the redirect — the user lands on `/login` with no `?redirect=` param, so they go to the root path after login instead of the page they tried to access.
3. The TanStack Router best practice (and this project's own routing.md lines 358-397) is to use `beforeLoad` on a `_authenticated` layout route so the check is synchronous, occurs before any loader, and can embed the return URL in the redirect.
4. The root uses `createRootRoute()` with an empty context `context: {}`, so auth state cannot be safely threaded through `beforeLoad` context even if desired.

**Recommendation:** Create `src/routes/_authenticated.tsx` with `beforeLoad: ({ context, location }) => { if (!context.auth.isAuthenticated) throw redirect({ to: '/login', search: { redirect: location.href } }) }`. Pass auth from a wrapper component via `<RouterProvider context={{ auth }} />`. Update `__root.tsx` to use `createRootRouteWithContext<{ auth: AuthState }>()`; then move all currently 'authenticated' routes under `_authenticated/`.

> **Verifier correction:** The finding is real and correctly described. The severity should be downgraded from high to medium: the "child route loaders failing before the auth redirect fires" sub-claim is overstated — most routes have no loaders, and the one layout that does (`/admin`) independently uses `beforeLoad`. The actual impact is: (1) a loading spinner flash before the redirect fires on first load, and (2) users landing at `/` after login instead of their intended page. These are UX problems, not security holes or data-loading failures, making this medium severity rather than high.

#### [high] String template literal navigation bypasses type safety

**Files:** `apps/frontend/src/features/tasks/components/create-task/create-code-task.tsx:215`, `apps/frontend/src/features/tasks/components/create-task-form.tsx:71`, `apps/frontend/src/features/tasks/hooks/use-compose-task-form.ts:312`

Three places construct navigation destinations using JavaScript template literals instead of typed `to` + `params` arguments. `navigate({ to: \`/tasks/${taskId}\` })` is an untyped string at runtime — TypeScript cannot verify the route path exists, that the param name matches `$task-id`, or that no required params are missing. If the route path ever changes (e.g., from `/tasks/$task-id` to `/t/$task-id`), these calls silently produce 404s with no compile-time error. TanStack Router's whole value proposition is type-safe navigation.

**Recommendation:** Replace with `navigate({ to: '/tasks/$task-id', params: { 'task-id': taskId } })`. This produces a TypeScript error if the route path or param name changes, giving a compile-time safety net instead of a runtime 404.

> **Verifier correction:** The finding is accurate as stated. No correction needed. All three locations use template literals that widen to `string` and bypass TanStack Router's compile-time route validation. The fix is to use the typed form: `navigate({ to: '/tasks/$task-id', params: { 'task-id': data.taskId } })` which the registered router would validate against the known route tree.

#### [high] Unsafe `as` casts in validateSearch on OAuth/auth callback routes

**Files:** `apps/frontend/src/routes/auth/callback.tsx:5-8`, `apps/frontend/src/routes/oauth/github/callback.tsx:10-13`, `apps/frontend/src/routes/oauth/figma/callback.tsx:10-13`

Three routes implement `validateSearch` by casting the raw object with `(search as { code?: string })` instead of using a Zod schema. This silently passes any value through without parsing or validation. If an attacker or a broken OAuth provider sends a malformed or missing `code`/`state`/`success` param, the component receives an unvalidated `unknown` value typed as a safe type. The project's own guideline (routing.md line 158-176) and the existing pattern in other routes (using Zod `.catch()`) both show the correct approach. The `as` cast is explicitly listed as the anti-pattern: `validateSearch: (raw) => raw as ...`.

**Recommendation:** Replace raw casts with Zod schemas using `.catch()`: `const callbackSearchSchema = z.object({ success: z.string().optional().catch(undefined) })` then `validateSearch: callbackSearchSchema`. This guarantees type safety and resilience against malformed URLs.

> **Verifier correction:** The issue is real but severity is inflated from high to low/medium. The `as` casts are inconsistent with the project's own Zod-based pattern and violate the frontend-conventions rule against unsafe casts, but they pose no meaningful security risk: (1) all three components already validate at runtime that the values are present before using them, (2) the parameters originate from trusted OAuth provider redirects and are strings by definition, (3) any attacker controlling these URL parameters still cannot bypass server-side OAuth validation. The correct fix is to replace the `as` casts with Zod schemas (e.g., `z.object({ code: z.string().optional(), state: z.string().optional() })`) for consistency, but this is a maintainability/style issue, not a security vulnerability.

#### [medium] No route-level code splitting (zero .lazy.tsx files)

**Files:** `apps/frontend/src/routes/admin/triggers/index.tsx`, `apps/frontend/src/routes/admin/triggers/$trigger-id/index.tsx`, `apps/frontend/vite.config.ts`

None of the 28 route files use TanStack Router's `.lazy.tsx` code-splitting convention. Particularly heavy routes like `/admin/triggers/` (270 lines with complex table logic, multiple imports) and `/admin/triggers/$trigger-id/` (257 lines) are eagerly bundled into the initial JS payload. The Vite config manually splits vendor libraries (React, TanStack, Radix) via `manualChunks`, but all route components land in the same app chunk. TanStack Router's Vite plugin supports automatic code splitting by co-locating route definition in `route.tsx` and component in `route.lazy.tsx`.

**Recommendation:** For admin and infrequently-visited routes, split the component into a `.lazy.tsx` file: rename component logic to `admin/triggers/index.lazy.tsx` exporting `createLazyFileRoute('/admin/triggers/')({ component: TriggersListPage })`, and keep only the route definition (validateSearch, loader, beforeLoad) in `index.tsx`. Start with the larger admin route files.

> **Verifier correction:** The finding is real but slightly overstates the impact. The most "heavy" routes (`/admin/triggers/`) are admin-only pages unlikely to be hit on initial load by most users. The `tasks/$task-id` route — which users hit most frequently and which transitively imports the largest components (chat, preview, sandbox diagnostics) — is only 31 lines in the route file but still eagerly bundles all its feature imports. The severity of "medium" is correct: code splitting would improve initial JS payload size, but since the app structure delegates rendering logic to `features/`, the route files themselves are not the source of bundle bloat. The real fix is enabling `autoCodeSplitting: true` in the `tanstackRouter()` call in `/opt/moatless/workspace/moatless-vibe/apps/frontend/vite.config.ts`, which is supported by the installed plugin version (1.127.9).

#### [medium] Duplicate dashboard search schema defined three times

**Files:** `apps/frontend/src/routes/index.tsx:11-18`, `apps/frontend/src/routes/dashboard.tsx:8-15`, `apps/frontend/src/routes/tasks/index.tsx:8-15`

The same six-field Zod search schema (view, tag, createdBy, repositoryId, status, page) is copy-pasted verbatim into three route files under different local names (`indexSearchSchema`, `dashboardSearchSchema`, `tasksSearchSchema`). Any change — adding a field, adjusting a `.catch()` default, or fixing a validation rule — must be applied to all three files. The codebase already has a precedent for shared schemas: `taskSearchSchema` is correctly extracted to `features/tasks/types/task-search-schema.ts` and imported by two routes.

**Recommendation:** Extract a shared schema to `features/tasks/types/dashboard-search-schema.ts` (matching the `taskSearchSchema` precedent) and import it in all three route files. This also ensures the `DashboardSearchParams` interface in `task-filters.tsx` stays structurally in sync with the validated schema.

> **Verifier correction:** No correction needed. The finding is real, the file locations and line numbers are accurate, the described schema fields match exactly, and the severity (medium) is appropriate for a maintainability/DRY violation with no current runtime impact.

#### [medium] Dashboard filter navigation replaces all search params instead of merging

**Files:** `apps/frontend/src/routes/index.tsx:33-35`, `apps/frontend/src/routes/dashboard.tsx:28-30`, `apps/frontend/src/routes/tasks/index.tsx:28-30`

The `handleSearchChange` callback on all three dashboard routes calls `navigate({ search: params })` where `params` is the full `DashboardSearchParams` object constructed in `task-filters.tsx`. This replaces the entire search string in one shot. While this works because `DashboardSearchParams` covers all fields, it establishes the anti-pattern of non-updater navigation. If any future code adds a search param outside `DashboardSearchParams` (e.g., a modal state or feature flag param), it will be silently dropped on every filter change. The TanStack Router best practice (and how use-task-view.ts correctly does it) is `search: (prev) => ({ ...prev, ...params })`.

**Recommendation:** Change to `navigate({ search: (prev) => ({ ...prev, ...params }) })` in all three dashboard `handleSearchChange` callbacks. This future-proofs against additional search params and aligns with the pattern already used in `use-task-view.ts`.

> **Verifier correction:** The pattern is a genuine anti-pattern inconsistent with how the same codebase handles search param updates elsewhere (use-task-view.ts), and the fix recommended (updater function) would be correct and safe. However, severity should be downgraded from medium to low: the `validateSearch` schemas at these routes are co-extensive with `DashboardSearchParams`, so TanStack Router would strip any future out-of-schema param regardless of which navigation form is used. The practical risk only materialises if someone simultaneously extends the route schema AND adds a UI element that passes data outside `DashboardSearchParams` to `handleSearchChange` — a double-oversight that is easy to catch at the point of change. The finding is real as a consistency/best-practice issue, not a current data-loss bug.

### Refuted claims (checked and dismissed)

- **No errorComponent on any route — no Retry path for loader failures** — The claim is substantially wrong on its most important assertion. The auditor states users see "a blank screen with no recovery path" if errors occur, but this is directly refuted by the code. In `__root.tsx`, the `RootComponent` wraps all non-public routes in a custom `<ErrorBoundary>` (imported from `@/lib/components/error-boundary`). That `ErrorBoundary` component is a full React class component with: a styled error UI using `ErrorAlert`, a "Try Again" button that calls `this.reset()` to clea

---

## performance — score 5/10

The frontend performs well in several areas — SSE event deduplication, optimistic cache updates that avoid polling, sensible pagination (not virtualized infinite lists), and a clearly structured code-splitting intent in vite.config.ts. However, several concrete performance problems exist: the @pierre/diffs code-path is imported in main.tsx and not isolated to the files feature; unused heavy packages (mermaid 66 MB, react-syntax-highlighter 12 MB) remain in package.json and in the vendor-markdown manualChunk despite no production imports; the dashboard renders up to 20 un-memoized TaskCards each with an independent SSE subscription that churns on every event; and the React Compiler (stable since Oct 2025) is not enabled despite the project using React 19 and @vitejs/plugin-react 4.6 which already supports it.

### Strengths
- SSE event handling avoids polling — sandbox status and task detail queries correctly disable refetchInterval and rely on server-sent events for real-time updates, minimizing redundant HTTP traffic.
- GlobalEventStreamProvider uses a compact Set-based deduplication ring (max 1000 IDs, trimmed to 500) to prevent reprocessing events on reconnect, which is a concrete guard against double-render storms.
- MessageHistory is wrapped in React.memo and its expensive groupSubagentMessages + buildRenderItems computations are memoized with useMemo, keeping the primary message stream efficient.
- Task lists use cursor-based pagination (PAGE_SIZE=20 in the dashboard, limit=100 in the sidebar) rather than rendering an unbounded list, which naturally caps DOM node count.
- useTaskActivity correctly bails out of setState and returns the same Map reference when nothing changed, preventing unnecessary re-renders of SidebarTasks during quiet periods.
- @pierre/diffs uses a Web Worker (DiffsWorker) for diff computation, offloading heavy syntax-highlighting work off the main thread.
- The manualChunks configuration in vite.config.ts groups stable vendor code into cacheable chunks (vendor-react, vendor-tanstack, vendor-radix, vendor-icons), reducing cache busting on app code changes.
- rollup-plugin-visualizer is listed as a devDependency, indicating bundle-size awareness even if it is not wired into the build script permanently.

### Confirmed findings

#### [high] @pierre/diffs imported unconditionally in main.tsx — loads on every page, not just the files feature

**Files:** `apps/frontend/src/main.tsx:1-2`

WorkerPoolContextProvider and DiffsWorker are imported at the application entry point, not lazily in the route or feature that uses them. This means every user — including those who only visit the dashboard or settings pages — downloads and parses the 7.3 MB @pierre/diffs package on first load. The WorkerPool worker is also spun up immediately, consuming memory before it is needed.

**Recommendation:** Move WorkerPoolContextProvider into the task route tree or the files feature entry. Either wrap it in React.lazy around the FileExplorer component, or create a dedicated ProviderWrapper that is code-split alongside the tasks/$task-id route. Example: const DiffProvider = lazy(() => import('./features/files/diff-provider')).

> **Verifier correction:** The finding is accurate as stated. No correction needed. The severity of high is appropriate for a 7.3 MB unconditional bundle addition at app startup. The recommendation to move `WorkerPoolContextProvider` into the files-feature route subtree (or to the specific route that renders the file viewer) and use a dynamic import for `DiffsWorker` would be valid in this codebase given its TanStack Router file-based routing structure.

#### [medium] manualChunks is configured as a static object rather than a function — Rollup cannot apply the bucketing to transitive imports, causing vendor code to leak into app chunks

**Files:** `apps/frontend/vite.config.ts:93-99`

The Rollup documentation (and Vite's own guidance) recommends using a function form for manualChunks when you want to bucket transitive dependencies. When written as a plain object, only the explicitly named top-level modules are moved into the specified chunk; their dependencies may be duplicated across other chunks or pulled into the main bundle. A function form receives each resolved module ID and returns the desired chunk name, giving you control over all transitive imports.

**Recommendation:** Replace the object form with a function:
manualChunks(id) {
  if (id.includes('node_modules/react') || id.includes('node_modules/react-dom')) return 'vendor-react';
  if (id.includes('@tanstack/react-query') || id.includes('@tanstack/react-router')) return 'vendor-tanstack';
  if (id.includes('@radix-ui')) return 'vendor-radix';
  if (id.includes('react-markdown') || id.includes('remark-gfm')) return 'vendor-markdown';
  if (id.includes('lucide-react')) return 'vendor-icons';
  if (id.includes('@pierre')) return 'vendor-pierre';
}

> **Verifier correction:** The code issue is real — `manualChunks` is written as a static object at vite.config.ts lines 93-99, and the function form would give more precise control over transitive dependency placement. However, severity should be downgraded from medium to low. Rollup does not duplicate transitive imports when using the object form; it creates shared chunks for them. The practical impact for this particular set of packages is minimal since they are relatively self-contained libraries. This is a best-practice improvement (switch to function form to ensure all transitive deps of each vendor are bucketed correctly) rather than an active performance bug causing measurable regression.

#### [medium] react-markdown receives a new components object on every render — defeats any internal memoization in ReactMarkdown

**Files:** `apps/frontend/src/features/files/components/markdown-preview.tsx:109-155`

MarkdownPreview passes an inline object literal to ReactMarkdown's components prop on every render. react-markdown v10 uses React.memo internally, but memo's shallow equality check always fails when components is a new object reference. This means every re-render of MarkdownPreview (triggered by file content updates or parent re-renders) forces react-markdown to re-render all its child renderers. The components object also embeds inline JSX callback functions for pre, code, table, and img.

**Recommendation:** Define the components object outside the component function body (or use useMemo if taskId/filePath must be captured in the img callback). For example, create MARKDOWN_COMPONENTS = { pre: ..., code: ..., table: ... } at module level and handle the img case separately with a memoized wrapper component that receives taskId/filePath as props.

#### [medium] React Compiler (stable since Oct 2025) is not enabled — all manual useMemo/useCallback/React.memo annotations could be compiler-managed

**Files:** `apps/frontend/vite.config.ts:64-66`

The project uses React 19.1 and @vitejs/plugin-react 4.6, both of which support the React Compiler. The compiler is not configured. Without it, developers must manually apply useMemo/useCallback/React.memo everywhere (79 occurrences across the tasks feature alone), and any missed annotation is a silent performance cliff. The compiler also applies finer-grained memoization than humans typically do.

**Recommendation:** Enable the React Compiler in vite.config.ts:
import react from '@vitejs/plugin-react';
plugins: [
  react({ babel: { plugins: ['babel-plugin-react-compiler'] } }),
  ...
]
Install: bun add -d babel-plugin-react-compiler. After enabling, audit components that read from mutable module-level objects (like the eventsByTask Map mutated in place in global-event-stream-provider.tsx line 303-305) as the compiler will opt those out — convert them to useSyncExternalStore or React state.

> **Verifier correction:** The finding is real and severity is appropriate. One minor correction: @vitejs/plugin-react does not have a built-in `reactCompiler` option (the type definitions confirm this). Enabling the React Compiler requires installing `babel-plugin-react-compiler` and adding it to the `babel.plugins` array: `react({ babel: { plugins: [['babel-plugin-react-compiler', {}]] } })`. The auditor's description implies a simpler toggle exists in the plugin, which is slightly misleading but does not change the substance of the finding.

#### [medium] TaskStatusCell in task-table calls useSandboxStatus per row — triggers up to 20 parallel sandbox API requests on the dashboard table view

**Files:** `apps/frontend/src/features/tasks/components/dashboard/task-table.tsx:48-51`, `apps/frontend/src/features/tasks/components/dashboard/task-table.tsx:109`

The TaskStatusCell component calls useSandboxStatus(task.taskId, task) inline, which issues a GET /api/v1/sandbox/{taskId}/status query for each row. With PAGE_SIZE=20, opening the table view fires up to 20 simultaneous sandbox status requests. TaskCard on the cards view does the same thing. React Query deduplicates requests for the same taskId, but for 20 different tasks it issues 20 requests. This is compounded by the same pattern in the sidebar (limit=100 tasks).

**Recommendation:** The sandbox status is already updated optimistically via SSE events in global-event-stream-provider (lines 164-169). For list views, consider deriving the status indicator from the task's own agentState/environmentStatus fields that are returned by the task list endpoint (these are populated in the list response as seen in taskRowState in sidebar-tasks.tsx line 106-117), and only call useSandboxStatus when the task detail view is open. This would reduce the dashboard network waterfall from 20 requests to 0.

> **Verifier correction:** The core issue is real and the severity is appropriate. However, two details in the description are inaccurate: (1) The requests are a one-time burst on mount (not continuous), because refetchInterval defaults to false in useTaskSandboxStatusQuery. The burst recurs only when the staleTime (5 seconds) has elapsed. (2) The sidebar does NOT compound the problem — sidebar-tasks.tsx uses limit:100 in its task list query but never calls useSandboxStatus; it renders TaskStatusIcon with just the task object. The sidebar claim should be removed from the description.

### Refuted claims (checked and dismissed)

- **mermaid (66 MB source) and react-syntax-highlighter (12 MB source) are declared as dependencies but never imported in production code — yet they appear in the vendor-markdown bundle chunk** — The claim contains one factual error (mermaid) and one partially-real finding (react-syntax-highlighter) with an incorrect mechanism description.

**Mermaid — claim is wrong**: The auditor claims "zero production imports of mermaid" and that the test mock is the only reference. This is false. The frontend imports `streamdown` in production code at `apps/frontend/src/lib/components/ai-elements/response.tsx`, and `streamdown`'s bundled `dist/index.js` contains `await import("mermaid")` as a dynami

- **Up to 20 un-memoized TaskCards each maintain their own SSE subscription — every event re-renders the entire dashboard card grid** — The audit contains a critical factual error about how events are dispatched. The global SSE provider uses a single connection (not 20), and the `notifySubscribers` function at line 261 of global-event-stream-provider.tsx does `taskHandlersRef.current.get(event.taskId)` — it dispatches only to handlers registered for the specific task that generated the event. An event for task A only invokes the handler in the TaskCard showing task A, not all 20 task cards' handlers. The claim "up to 20 setRealt

### Low-severity notes (not independently verified)
- rollup-plugin-visualizer is a devDependency but is not wired into the vite.config.ts build — bundle size regressions go undetected

---

## error-handling — score 5/10

The frontend error handling is competent but incomplete. The API client correctly normalizes errors into typed ApiError objects, QueryClient centralizes auth redirects and server-status detection, and the task page uses well-granulated ErrorBoundaries around its major panels. However, the single root ErrorBoundary sits inside AuthGate rather than wrapping the whole tree, there is no MutationCache-level global handler so mutation errors on several fire-and-forget calls are silently discarded, no error monitoring service is integrated (componentDidCatch is a commented-out stub), and the query client sets retry: false globally which eliminates the ability to distinguish retriable 5xx from non-retriable 4xx errors. A number of empty catch blocks in critical paths (event stream, chat cards) also swallow errors without any user or log feedback.

### Strengths
- Typed ApiError class in src/lib/api/client.ts correctly checks response.ok and throws a structured error object, preventing silent HTTP failures from leaking into TanStack Query as successes.
- QueryCache onError handler in query-client.ts centralizes 401 redirect logic with a cooldown guard, and 503/network-error detection feeds the global ServerDownGate with a useSyncExternalStore pattern.
- Task page (task-page.tsx) uses four distinct feature-level ErrorBoundaries — File Explorer, Sandbox Controls, Preview, Chat View — preventing any single panel crash from unmounting the whole task UI.
- The ErrorBoundary class itself supports resetKeys, a custom fallback render-prop, isolate mode, and optional componentDidCatch callbacks, giving callers a rich reset API.
- Mutations that own critical state (useSendMessage, useToggleTaskFollow, useCloseTask, useDeleteMessage) implement proper optimistic-update rollback in their onError handlers.
- The FullPageError and ErrorAlert components provide status-code-aware messaging (404, 403, 5xx) with retry affordances, and are actually wired to the task and dashboard error paths.

### Confirmed findings

#### [critical] No error monitoring integration — componentDidCatch is a TODO stub

**Files:** `apps/frontend/src/lib/components/error-boundary.tsx:150`

The componentDidCatch body contains only a comment: '// Example: sendToErrorTrackingService(error, errorInfo)'. No error monitoring service (Sentry, Datadog, etc.) is wired in either componentDidCatch or the QueryCache onError handler. Production errors that reach error boundaries or background query failures are never captured with stack traces, component trees, or query keys, making production incidents extremely hard to diagnose.

**Recommendation:** Integrate an observability SDK (e.g. Sentry) at startup in main.tsx, then call captureException(error, { extra: errorInfo }) inside componentDidCatch and Sentry.captureException(error, { extra: { queryKey: query.queryKey } }) inside the QueryCache onError callback.

> **Verifier correction:** The finding is accurate: componentDidCatch is indeed a stub with no error monitoring wired in, and the QueryCache onError handler only handles auth/availability — not error reporting. However, the severity should be "high" rather than "critical". This is a self-hosted developer tool where operators have access to server-side logs and browser DevTools; the absence of a third-party error monitoring service is a significant observability gap but not a critical security or data-integrity issue. The recommended fix (integrating Sentry or a similar service, or at minimum logging errors to console.error in production) is valid for this codebase.

#### [high] No MutationCache global onError handler — several mutations silently discard errors

**Files:** `apps/frontend/src/lib/api/query-client.ts:49`, `apps/frontend/src/features/tasks/components/task-header/task-info-section.tsx:43`, `apps/frontend/src/features/navigation/components/sidebar-tasks.tsx:204`, `apps/frontend/src/features/tasks/components/chat-view/chat-container.tsx:270`

The QueryClient is constructed with a QueryCache but no MutationCache. As a result, mutations that are called without an onError option produce no user-visible feedback on failure. The sidebar's closeTask.mutate() (line 220), task-info-section's updateVisibility.mutate() and updateTtl.mutate(), and chat-container's stopAgent.mutate() and deleteMessage.mutate() all fall into this gap. When these fail, the optimistic UI rollback runs silently and the user has no indication that their action failed.

**Recommendation:** Add a MutationCache({ onError }) to the QueryClient that shows a generic toast for any mutation error not already handled by the caller's own onError option. Check query.state.data !== undefined as a guard where appropriate. Alternatively, add onError toast handlers to each of the affected call sites.

> **Verifier correction:** The issue is real but severity should be medium, not high. The auditor's evidence quote is inaccurate: there IS a defaultOptions.mutations.onError (it just handles auth/server-status only, not user toasts). Also, task-info-section's closeTask DOES have an explicit onError toast — the auditor incorrectly listed it as a gap. The actual silent-failure mutations are: (1) sidebar-tasks.tsx's closeTask.mutate() at line 220 — no onError toast, (2) task-info-section's updateVisibility.mutate() at line 95 — no onError toast, (3) task-info-section's updateTtl via Select onValueChange at line 168 — no onError toast, (4) chat-container's stopAgent (line 270) — no onError and no hook-level error handler, (5) chat-container's deleteMessage (line 288) — hook has rollback but no toast. These are real UX gaps where optimistic rollback happens silently, but they do not cause silent data corruption — the UI state does recover correctly. The fix (adding onError toasts at call sites, or adding a MutationCache with a generic toast) is valid and would improve UX, but "high" overstates the impact.

#### [high] Root-level ErrorBoundary sits inside AuthGate — public route crashes are uncaught

**Files:** `apps/frontend/src/routes/__root.tsx:120`, `apps/frontend/src/routes/__root.tsx:133`

In RootComponent, the public route branch (login, /auth/*) renders Outlet directly under ServerDownGate with no ErrorBoundary. The non-public branch wraps Outlet in an ErrorBoundary but that boundary is inside AuthGate, so any render error thrown by AuthGate itself, by FeatureFlagsProvider, by QueryClientProvider, or by WorkerPoolContextProvider (all mounted in main.tsx above the router) will reach the root React error boundary with an unformatted white screen rather than a meaningful fallback.

**Recommendation:** Wrap the RouterProvider in main.tsx with a top-level ErrorBoundary so the entire application tree has a last-resort boundary. Also add an explicit ErrorBoundary around the public route Outlet inside RootComponent.

> **Verifier correction:** The finding is real but the scope description is partially overstated. The two genuine issues are: (1) public routes lack an ErrorBoundary, and (2) the ErrorBoundary is inside AuthGate rather than wrapping it, so AuthGate's own render errors escape uncaught. The claim that FeatureFlagsProvider/QueryClientProvider/WorkerPoolContextProvider errors "will reach the root React error boundary with an unformatted white screen rather than a meaningful fallback" is accurate as a description of what happens, but is misleading as a critique of the ErrorBoundary placement in __root.tsx — moving the ErrorBoundary in __root.tsx would not fix those providers since they live above the router in main.tsx. A separate ErrorBoundary wrapping the providers in main.tsx would be needed for those. The two cited issues (no boundary on public routes, boundary inside rather than outside AuthGate) are the accurate actionable findings at the claimed severity.

#### [high] global retry: false disables automatic retry for transient 5xx/network errors

**Files:** `apps/frontend/src/lib/api/query-client.ts:64`

The QueryClient defaultOptions sets retry: false for all queries. This means a single transient network hiccup or a momentary 500 from the backend causes every query to permanently error. TanStack Query's built-in retry with exponential backoff (default 3 retries) is specifically designed for transient failures. The current setting forces every failed request to land immediately in the error UI, degrading resilience unnecessarily.

**Recommendation:** Replace retry: false with a discriminating function: retry: (count, error) => !(error instanceof ApiError && error.status < 500) && count < 2. This retries on network errors and 5xx, but not on 4xx (which are non-retriable client errors). Keep retry: false on mutations, where retrying user-initiated writes is unsafe.

> **Verifier correction:** The issue is real but the severity should be downgraded from high to medium/low. The `retry: false` is a documented, intentional design decision: 503 and network errors are handled via the global `serverStatusStore`/`ServerDownGate` mechanism (server-status.ts lines 8-10 state this explicitly). The only genuine gap is that transient 500/502/504 errors go immediately to error UI without retries. Given the caching (staleTime: 5 min) and the explicit team intent visible in the code comments and per-hook settings, this is better classified as a deliberate tradeoff than an unintentional resilience failure. If this is to be changed, the fix should be a discriminating retry function (e.g., `retry: (count, error) => count < 2 && isRetryableStatus(error)` that targets 500/502/504 specifically) rather than restoring the blanket default, since 4xx errors and auth failures must not retry.

#### [medium] Dashboard and Settings/Secrets pages lack feature-level ErrorBoundaries

**Files:** `apps/frontend/src/routes/dashboard.tsx`, `apps/frontend/src/routes/secrets.tsx`, `apps/frontend/src/features/tasks/components/dashboard/task-dashboard.tsx`

The dashboard route renders TaskDashboard with no wrapping ErrorBoundary. If TaskDashboard or any of its child components (TemplateChipsRow, CreateCodeTask, TaskCard, TaskTable) throws a render error, it escalates to the single root boundary which unmounts the entire authenticated app. The settings, secrets, and admin routes similarly have no feature-level boundaries. By contrast, the task detail page has four well-placed boundaries.

**Recommendation:** Wrap each major route's component in an ErrorBoundary at the route level, either in the route file itself or in the root RootComponent using TanStack Router's errorComponent feature (which is the idiomatic approach for TanStack Router v1).

> **Verifier correction:** The finding is real and the severity (medium) is appropriate. However, the claim that "settings" routes lack feature-level boundaries is inaccurate: routes/settings.tsx is a redirect-only route with no rendered component. The admin routes (routes/admin/index.tsx, routes/admin/commands/index.tsx, etc.) are the correct additional targets, not settings.tsx. The core claim about dashboard.tsx and secrets.tsx is fully confirmed.

#### [medium] No global window.unhandledrejection handler for floating promises

**Files:** `apps/frontend/src/main.tsx`

There is no window.addEventListener('unhandledrejection') handler registered anywhere in the application. The global event stream provider's fetch() call resolves via .then()/.catch() chains, and several places call mutate() (not mutateAsync) so errors flow through the mutation's onError if provided. However, any Promise that escapes a try/catch block without being returned or awaited will produce an unhandled rejection that is completely invisible in production. Without this last-resort handler, such errors can never reach the error boundary or any monitoring service.

**Recommendation:** Add to main.tsx: window.addEventListener('unhandledrejection', (event) => { console.error('Unhandled promise rejection:', event.reason); /* sentry.captureException(event.reason) */ }) to ensure floating rejections are at minimum logged.

> **Verifier correction:** The finding is real but slightly overstated in scope. The global-event-stream-provider's fetch() call does have a .catch() handler and the processStream async function is wrapped in try/catch, so those specific paths do not produce unhandled rejections. The core gap — no last-resort window.unhandledrejection handler and no error monitoring service — is genuine and the severity of 'medium' is appropriate as written.

### Refuted claims (checked and dismissed)

- **Empty catch blocks in high-traffic code paths (event stream, chat cards)** — The auditor's core claim — that all three call sites suppress errors with no user feedback — is incorrect for two of the three files.

For ask-user-question-card.tsx (line 47): The catch block exists but the component renders `submitMutation.error` at lines 171-177 with a destructive-styled error message. The catch simply prevents an unhandled promise rejection while the mutation's own error state provides UI feedback. This is a correct TanStack Query pattern, not a bug.

For plan-approval-card.

### Low-severity notes (not independently verified)
- QueryCache onError does not distinguish stale-data refetch failures — background failures get no toast

---

## components — score 6/10

The frontend codebase is well-structured with strong adherence to feature-based module boundaries, consistent named exports, good context safety guards, and a working shadcn/ui compound component pattern in SidebarDialog. However, all 18 Context.Provider usages use the deprecated `.Provider` form (React 19 canonicalises `<Context value={...}>`), the two most critical providers (ThemeProvider and FeatureFlagsProvider) create new inline object values on every parent render without memoization, and the lib/components/chat sub-tree has a pervasive architectural inversion where "shared" components import directly from feature internals, making them non-reusable outside the tasks feature. Several components significantly exceed the 200-line guideline (chat-container.tsx at 745 lines, assistant-message.tsx at 827 lines, preview.tsx at 798 lines).

### Strengths
- Zero usage of forwardRef — the codebase never wraps components in forwardRef(), fully aligned with React 19's direct ref prop model.
- All contexts guard against out-of-provider usage with a descriptive thrown Error in a dedicated custom hook (useTaskDataContext, useChatToggle, useGlobalEventStream, etc.), satisfying the industry best-practice of never silently misbehaving.
- Consistent named exports throughout all production source files; default exports are only used in Storybook meta objects (which require them) and test utilities.
- Feature index files (features/tasks/index.ts, features/events/index.ts, etc.) act as proper public-API boundaries, and most cross-feature imports go through these barrel files rather than deep internal paths.
- Context values in computationally heavy providers (TaskDataProvider, ComposeTaskProvider, GlobalEventStreamProvider, TaskUIProvider, SidebarProvider) are memoized with useMemo and callbacks with useCallback, preventing unnecessary consumer re-renders.
- The compound component pattern (SidebarDialog with .Sidebar, .Search, .Nav, .NavItem, .Footer, .Content) is cleanly implemented using Object.assign and a root context, avoiding prop explosion.
- Route files (src/routes/) correctly serve as thin orchestrators that extract URL params and delegate rendering to feature components, matching the project's own architecture guideline.
- Index-as-key is only used for static skeleton placeholder lists with a biome-ignore annotation explaining the rationale; no dynamic data lists use index as key.

### Confirmed findings

#### [medium] All contexts use deprecated Context.Provider form instead of React 19 direct context-as-provider

**Files:** `apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx:643`, `apps/frontend/src/features/tasks/contexts/task-ui-provider.tsx:71`, `apps/frontend/src/features/tasks/components/task-page-layout.tsx:103`, `apps/frontend/src/features/tasks/components/task-page-layout.tsx:139`, `apps/frontend/src/features/tasks/contexts/task-data-provider.tsx:68`, `apps/frontend/src/features/tasks/contexts/compose-task-provider.tsx:94`, `apps/frontend/src/lib/components/theme-provider.tsx:63`, `apps/frontend/src/lib/components/ui/sidebar-dialog.tsx:58`, `apps/frontend/src/lib/components/ui/sidebar.tsx:201`, `apps/frontend/src/lib/contexts/feature-flags-context.tsx:74`

All 18 provider render calls use the deprecated `<Context.Provider value={...}>` syntax. React 19 deprecated `.Provider` in favour of rendering the context object directly (`<Context value={...}>`). Using the deprecated form adds an unnecessary JSX nesting level, and React will emit a deprecation warning in future minor versions.

**Recommendation:** Replace every `<FooContext.Provider value={v}>` with `<FooContext value={v}>`. This is a mechanical find-and-replace; no logic changes are needed. The official React 19 codemod (`npx codemod@latest react/19/replace-context-provider`) can automate this across the entire codebase in one step.

> **Verifier correction:** The finding is real but severity should be LOW, not medium. The `Context.Provider` form still works identically in React 19.1.0, produces no deprecation warnings today, and is a purely cosmetic/future-proofing change. The scope (18 occurrences) is accurate. The recommendation to switch to `<Context value={...}>` is valid as a low-priority cleanup, but does not warrant medium severity.

#### [high] ThemeProvider and FeatureFlagsProvider create a new inline context value object on every render, causing all consumers to re-render unnecessarily

**Files:** `apps/frontend/src/lib/components/theme-provider.tsx:52`, `apps/frontend/src/lib/contexts/feature-flags-context.tsx:60`

ThemeProvider builds its context value as a plain inline object literal (`const value = { theme, setTheme: ... }`) on every render, including an inline arrow function for setTheme. FeatureFlagsProvider passes `{ flags, isLoading, isEnabled, getSlackConfig }` inline where `isEnabled` and `getSlackConfig` are also recreated on every render as closures. Because object identity changes on each parent render, every consumer of `useTheme()` or `useFeatureFlagsContext()` re-renders even when the theme or flags have not actually changed. These are app-wide providers that sit near the root, so every keystroke or unrelated state change in their ancestor propagates to all consumers.

**Recommendation:** Wrap the context value in `useMemo` and each callback in `useCallback`. For ThemeProvider: `const setTheme = useCallback((newTheme) => { ... }, [storageKey])` and `const value = useMemo(() => ({ theme, setTheme }), [theme, setTheme])`. For FeatureFlagsProvider: `const isEnabled = useCallback((flag) => { ... }, [flags])`, `const getSlackConfig = useCallback(() => flags?.slack ?? undefined, [flags])`, and `const value = useMemo(() => ({ flags, isLoading, isEnabled, getSlackConfig }), [flags, isLoading, isEnabled, getSlackConfig])`.

> **Verifier correction:** The finding is real but the severity is wrong. The missing memoization is a genuine pattern issue (no `useMemo`/`useCallback` in either provider), but severity should be **low**, not high. ThemeProvider has exactly 1 consumer and only re-renders on explicit user theme changes. FeatureFlagsProvider's state stabilizes after initial load and never changes again, so in practice the inline object recreation causes at most one extra consumer re-render in the app's lifetime. The auditor's claim that "every keystroke or unrelated state change in their ancestor propagates to all consumers" is factually wrong — React Context consumers only re-render when the Provider re-renders, which requires the Provider's own state or props to change, not changes in arbitrary ancestors.

#### [high] lib/components/chat imports from feature internals, breaking the shared-component architecture boundary

**Files:** `apps/frontend/src/lib/components/chat/message-history/plan-approval-card.tsx:3`, `apps/frontend/src/lib/components/chat/message-history/ask-user-question-card.tsx:3`, `apps/frontend/src/lib/components/chat/command-autocomplete/command-autocomplete.tsx:3`, `apps/frontend/src/lib/components/chat/message-history/assistant-message.tsx:17`, `apps/frontend/src/lib/components/chat/message-history/message-history.tsx:15`, `apps/frontend/src/lib/components/chat/chat-input/selectors/hooks.ts:3`, `apps/frontend/src/lib/components/error-boundary.tsx:4`, `apps/frontend/src/lib/layout/index.tsx:3`

The project's own guidelines state that `lib/components/` should hold 'Generic, reusable UI components'. However, lib/components/chat and lib/layout import directly from feature internals: plan-approval-card and ask-user-question-card call `useTaskId()` and `useSendMessage()` from features/tasks, command-autocomplete.tsx imports getCommandIcon from features/tasks/components/chat-view/command-buttons, and lib/components/error-boundary imports CreateTaskDialog. This creates a circular dependency direction (lib depends on features) and means these 'shared' components silently fail outside a TaskProviders context. The error boundary has a hardcoded repository UUID ('0ee846ce-fabd-4b90-9ec5-def401a5d802') as a default prop fallback, which is a maintenance hazard.

**Recommendation:** Move the task-coupled chat components (plan-approval-card, ask-user-question-card) into features/tasks/components/ since they are only meaningful within that feature context. Pass task-specific callbacks (onApprove, onReject, taskId) as props from the feature layer rather than calling task hooks directly in lib. For command-autocomplete, accept an iconResolver prop instead of importing from features/tasks. For lib/layout, pass ComposeDock as a children prop or use a slot pattern. Remove the hardcoded UUID from error-boundary.tsx.

> **Verifier correction:** The issue is real and confirmed across all cited files. The hardcoded UUID hazard is somewhat overstated: it lives in a debug-only code path (`moatlessDebugMode` localStorage flag) and is never actually passed through the ErrorBoundary's own render method, reducing its practical risk. The severity should be adjusted from high to medium: the architectural boundary violation is genuine and widespread, but it is not causing any runtime failures — it is a maintainability and reusability concern. The lib/components/chat sub-tree and lib/layout are effectively task-feature-specific components misplaced under the shared lib namespace, not general-purpose components, and should either be moved to features/tasks or refactored to accept hooks/callbacks as props.

#### [medium] ChatToggleContext passes an unmemoized inline object as its context value, causing all useChatToggle() consumers to re-render on every TaskPageLayout render

**Files:** `apps/frontend/src/features/tasks/components/task-page-layout.tsx:103`, `apps/frontend/src/features/tasks/components/task-page-layout.tsx:139`

TaskPageLayout provides ChatToggleContext with `value={{ isChatCollapsed, toggleChat }}` — a new object literal created on each render. While `toggleChat` is stable (wrapped in useCallback), the object wrapper itself is new on every render, causing every consumer of `useChatToggle()` to re-render even when neither `isChatCollapsed` nor `toggleChat` has changed. Additionally, the provider appears twice (once in the mobile branch, once in the desktop branch), duplicating the risk.

**Recommendation:** Add a `useMemo` call: `const chatToggleValue = useMemo(() => ({ isChatCollapsed, toggleChat }), [isChatCollapsed, toggleChat])` and use `<ChatToggleContext value={chatToggleValue}>` in both branches. This also gives an opportunity to use the React 19 direct context syntax at the same time.

> **Verifier correction:** The issue is real and the code exactly matches the claim. However, severity should be low rather than medium: there is exactly one consumer (task-kiosk-header.tsx), it only uses the stable `toggleChat` callback (not `isChatCollapsed`), and the component is a small header. The "duplicated risk" from two Provider branches is not additive — only one branch renders at a time. The pattern is still worth fixing with `useMemo` wrapping the value object, but the actual performance impact is minimal in the current codebase.

#### [medium] ChatContainer and TaskPage are God Components exceeding 400-700 lines with multiple distinct responsibilities

**Files:** `apps/frontend/src/features/tasks/components/chat-view/chat-container.tsx:1`, `apps/frontend/src/features/tasks/components/task-page.tsx:1`, `apps/frontend/src/lib/components/chat/message-history/assistant-message.tsx:1`, `apps/frontend/src/features/preview/components/preview.tsx:1`

ChatContainer (745 lines) handles: message fetching, autocomplete (file + command), sandbox status polling, send/stop/delete mutations, queued message management, pending user input detection, agent mode + model selection, Figma imports, command execution by name, and layout rendering. Biome had to be suppressed three times for excessive cognitive complexity. TaskPage (426 lines) manages startup events, sandbox events, context items, element inspector state, preview navigation callbacks, chat toggle callbacks, and view routing. The project's own guideline (component-patterns.md) prescribes splitting at >200 lines. assistant-message.tsx (827 lines) and preview.tsx (798 lines) are similarly oversized.

**Recommendation:** Extract the state and mutation logic from ChatContainer into a dedicated `useChatController` hook, and break the render section into smaller sub-components: <ChatInputArea>, <SandboxErrorBanner>, <PendingInputArea>. For TaskPage, move the sandbox event accumulation logic into a `useSandboxEventLog(taskId)` hook. For assistant-message.tsx, split the tool call rendering into its own <ToolCallSection> component. Each extracted unit will be testable independently and the files will stay under the 200-line guideline.

#### [medium] FeatureFlagsProvider bypasses TanStack Query and uses raw fetch + useEffect for a server-state concern

**Files:** `apps/frontend/src/lib/contexts/feature-flags-context.tsx:28`

FeatureFlagsProvider fetches feature flags with a raw `apiClient.get()` inside a `useEffect`, managing loading and data state manually with `useState`. The project's own frontend conventions explicitly state 'Server state lives in TanStack Query — never duplicate in React useState'. Using useEffect for data fetching is also an explicit project anti-pattern. The workaround (manually setting serverStatusStore) duplicates logic that TanStack Query's error handling provides natively.

**Recommendation:** Convert to `useQuery` with a `staleTime: Infinity` (flags don't change without a deploy) and a sensible `retry: 1`. Export the query key to allow targeted invalidation. Move the provider to just expose the TanStack Query result through context, or simply expose a `useFeatureFlags()` hook that calls `useQuery` directly and drop the Context entirely — since flags are static per page load, Context provides no caching benefit over a single shared query.

> **Verifier correction:** The issue is real but the severity and framing need adjustment. The useEffect/useState pattern is intentional and architecturally necessary because FeatureFlagsProvider is mounted outside QueryClientProvider in main.tsx — useQuery would throw because the QueryClient context is not available at that tree level. The correct characterization is: this is a deliberate architectural trade-off (bootstrap provider placed above the query layer), not an accidental convention violation. If the team wanted to use TanStack Query here, the fix would be to restructure main.tsx so FeatureFlagsProvider is nested inside QueryClientProvider, then replace the useEffect with useQuery. The severity should be 'low' rather than 'medium' since the behavior is intentional, documented with a comment, and the one-time nature of the fetch makes TanStack Query's caching/retry advantages marginal here.

### Low-severity notes (not independently verified)
- preview feature imports task-specific internal hooks directly rather than going through the tasks public index
- Forms use onSubmit + e.preventDefault() instead of React 19 native form action pattern

---

## state — score 6/10

The frontend manages state with generally sound practices — TanStack Query is used correctly for server state, Context providers are reasonably split, and the global event stream provider uses refs and subscriptions to minimize React re-renders. However, several recurring violations of the project's own derived-state guidelines exist: useEffect+setState to sync form fields from server data, a feature-flags bootstrap that duplicates a fetch into useState instead of TanStack Query, multiple `useEffect` calls used purely to fire side-effect notifications (toast errors) rather than from event handlers, and a debounced-search pattern implemented with two tightly-coupled state variables and an effect chain. The codebase scores well on cleanup (event subscriptions and SSE streams all return teardown functions) and on not placing TanStack Query data directly into useState, but the systematic useEffect-for-derived/sync-state pattern keeps the score moderate.

### Strengths
- TanStack Query is the sole owner of server state: no instances of useState(queryResult.data) were found, eliminating the most common React data-fetching anti-pattern.
- Global event stream (SSE) cleanup is correctly handled: every useEffect that opens a connection or registers a listener returns a teardown function, preventing memory leaks.
- The global query client is configured with staleTime: 5 minutes and refetchOnWindowFocus: false, preventing gratuitous background refetches for stable data.
- The task-data-provider and global-event-stream-provider use useRef for frequently-updated values (lastEvent, handler sets) and only expose stable primitives via context, keeping Context-driven re-renders minimal.
- Context providers are appropriately split — FeatureFlagsContext, ThemeProviderContext, GlobalEventStreamContext, TaskDataContext, and TaskUIContext each hold a single concern, not a God-object.
- Optimistic updates in mutations (useSendMessage, useCloseTask, useDeleteTask) follow the canonical TanStack Query pattern with snapshot, update, and rollback in onMutate/onError.
- The RepositoryConfigEditSection correctly separates server state (baseFormData from useMemo on query data) from user modifications (userModifications useState), avoiding direct server-state duplication.

### Confirmed findings

#### [high] useEffect+setState to synchronize form fields from server data (ProfileConfigurationSection)

**Files:** `apps/frontend/src/features/settings/components/profile-configuration-section.tsx:23-28`

The component initializes name/email as empty useState('') and then uses useEffect to copy query data into those state variables when the user object arrives. This is exactly the anti-pattern condemned in the project's own derived-state.md ('Storing What Can Be Calculated'). It produces an extra render after the user data loads, and if the useAuth query refetches in the background the local state will NOT update because the effect only fires when the user reference changes — leading to stale form values.

**Recommendation:** Use the key prop to reset the form when identity changes, or initialize the form field defaults inline from the query result. For a settings form that is meant to be edited, the correct approach is to render the form only after user is available and pass defaultValue from the server data, or use react-hook-form with defaultValues={{ name: user.name ?? '', email: user.email ?? '' }} and key={user.id} on the form so it re-mounts when the user changes. Remove the useEffect entirely.

> **Verifier correction:** The issue is real and the code evidence is accurate. Severity should be medium rather than high: the anti-pattern is genuine and the background-refetch-wiping-edits bug is real, but the failure window is narrow for typical auth data query behavior. The fix is to use lazy useState initialization (useState(() => user?.name ?? '')) instead of the useEffect sync, which removes both the extra render and the stale-edit risk without restructuring the component.

#### [high] FeatureFlagsProvider bypasses TanStack Query and duplicates fetch state in useState

**Files:** `apps/frontend/src/lib/contexts/feature-flags-context.tsx:28-58`

The provider uses a raw fetch inside useEffect with manual useState([flags, isLoading]) instead of useQuery. This opts out of all TanStack Query benefits: no caching, no deduplication with other callers, no background refetch on focus/reconnect, no DevTools visibility. The context value (flags, isLoading, isEnabled, getSlackConfig) is recreated on every render because it is not memoized, causing every consumer to re-render whenever the parent renders. Additionally, the entire function body (isEnabled, getSlackConfig) is re-created inline on every render instead of being stable callbacks.

**Recommendation:** Replace the raw fetch + useState with useQuery({ queryKey: ['feature-flags'], queryFn: ..., staleTime: Infinity }) and consume flags inside the context. Wrap the context value in useMemo. Move isEnabled and getSlackConfig to stable useCallback or standalone functions.

> **Verifier correction:** The finding is real but severity should be downgraded from "high" to "low-medium". The raw-fetch + useState pattern is intentional (documented in an inline code comment at line 38), runs only once at app startup, and there are no other callers to deduplicate with. The lack of background refetch is a feature, not a bug, for bootstrap configuration. The genuine issues — missing useMemo on the context value and missing useCallback on isEnabled/getSlackConfig — cause unnecessary re-renders but are practically inert since the Provider itself rarely re-renders after initial mount.

#### [high] useEffect used to sync state that is derived from the previewPath URL param (TaskPage)

**Files:** `apps/frontend/src/features/tasks/components/task-page.tsx:229-264`

contextItems is a local React state array. A useEffect watches previewPath (derived from the URL via usePreviewPath()) and calls setContextItems to add/update/remove the previewUrl context entry whenever the URL param changes. This is the 'Effect Chain / Sync-state-from-props' anti-pattern from derived-state.md. It causes an extra render after every navigation, and the update can race with other setContextItems calls (e.g., handleToggleElementContext). The previewUrl entry is derived data — it is always deterministically computable from previewPath and the rest of contextItems.

**Recommendation:** Eliminate the useEffect. Instead, compute the effective context items during render: const effectiveContextItems = useMemo(() => { if (!previewPath || previewPath === '/') return contextItems.filter(i => i.type !== 'previewUrl'); const existing = contextItems.findIndex(i => i.type === 'previewUrl'); ... return ...; }, [contextItems, previewPath]) and pass effectiveContextItems to ChatContainer.

> **Verifier correction:** The finding is real but the severity should be medium, not high. The anti-pattern causes an extra render on every `previewPath` change and introduces a subtle stale-closure window in `handleToggleElementContext` (which reads `contextItems` at call time to find element indices), but it does not produce visible data loss or incorrect behavior in typical usage paths. The fix is to split state: keep `contextItems` holding only user-toggled element items, then compute the final array with `useMemo` by appending the `previewUrl` entry when `previewPath` is non-root — eliminating the effect entirely.

#### [medium] useEffect fires toast error as a side effect of error state (multiple components)

**Files:** `apps/frontend/src/features/tasks/components/chat-view/chat-container.tsx:163-169`, `apps/frontend/src/features/tasks/components/create-task/create-code-task.tsx:112-118`, `apps/frontend/src/features/tasks/hooks/use-compose-task-form.ts:90-96`, `apps/frontend/src/features/tasks/hooks/use-compose-task-form.ts:169-175`

Multiple components/hooks use useEffect(() => { if (error) toast.error(...) }, [error]) to display error notifications when a query fails. React's guidance ('You Might Not Need an Effect') explicitly classes this as 'Notifying parent components about state changes' — one of the anti-patterns. The toast fires on every re-render where error is truthy, including StrictMode double-invocations in development, which can produce duplicate toasts.

**Recommendation:** Move the error toast into the query's onError callback (TanStack Query v5: throwOnError + meta.errorMessage pattern, or the query's meta.onError hook in the global QueryCache). Alternatively, use the useQuery's error return value to render an inline error message. Do not use useEffect to react to query error state.

> **Verifier correction:** The finding is real but the description slightly overstates the impact. The effect does NOT fire on every re-render — it fires only when the `error` reference changes, which TanStack Query makes stable (the same Error object reference is returned until the query is retried or reset). The practical issues are: (1) duplicate toasts in React StrictMode during development, and (2) toasts re-firing if the component unmounts and remounts while the query is in error state (since useEffect runs on mount). Severity "medium" is appropriate as written.

#### [medium] Debounced search implemented as an effect chain (two useState + useEffect driving each other)

**Files:** `apps/frontend/src/features/figma/components/canvas-search.tsx:114-133`

The component holds both searchQuery and debouncedQuery in separate useState calls, then uses a useEffect to copy the former into the latter with a 300ms delay. This is precisely the 'Effect Chain' anti-pattern from the project's derived-state.md. It produces an intermediate render between the user typing and the results updating, and the two state variables can temporarily diverge causing the UI to be inconsistent.

**Recommendation:** Replace the pattern with a single useState for the raw query and use a custom useDebounce hook (or useDeferredValue from React 18+) to derive the debounced value during render without an effect: const debouncedQuery = useDebounce(searchQuery, 300). There are also established small libraries (use-debounce) that handle this correctly without effects.

#### [medium] useEffect used to register an imperative callback (onCommandExecutorReady) instead of using a ref

**Files:** `apps/frontend/src/features/tasks/components/chat-view/chat-container.tsx:491-493`, `apps/frontend/src/features/tasks/components/task-page-layout.tsx:83-98`

Two components use useEffect to call a parent-supplied callback with an imperative function reference. In chat-container.tsx, onCommandExecutorReady?.(executeCommandByName) is fired in a useEffect whenever executeCommandByName changes. In task-page-layout.tsx, onChatToggleReady is called in a useEffect with a closure over chatPanelRef. The React team's 'You Might Not Need an Effect' document classifies this pattern ('Sending data to the parent') as an anti-pattern: it fires after render (causing a flicker window), re-fires every time the callback identity changes, and makes data flow hard to follow.

**Recommendation:** Use useImperativeHandle (if the child needs to expose an API to a parent ref) or pass the callback as a regular prop to a child that stores it in a ref itself. For the command executor pattern, use React.forwardRef + useImperativeHandle({ executeCommand }) on ChatContainer so the caller can call chatRef.current.executeCommand(name) directly.

#### [medium] Multiple chained useEffect+setState in github-configuration-section and figma-configuration-section for OAuth popup detection

**Files:** `apps/frontend/src/features/settings/components/github-configuration-section.tsx:164-213`, `apps/frontend/src/features/settings/components/figma-configuration-section.tsx:39-92`, `apps/frontend/src/features/figma/components/figma-import-dialog.tsx:50-83`

All three OAuth flow components use three separate useEffect calls that chain through isConnecting state: (1) attach window focus listener when isConnecting=true, (2) reset isConnecting when config.connected becomes true (setIsConnecting(false) inside effect), (3) poll popupRef.current.closed with setInterval. The second effect is a classic 'synchronizing state from another state variable' anti-pattern — it derives UI state from both isConnecting and the server's connected status, then calls setState to update. FigmaImportDialog additionally has a useEffect that mirrors mutation error state: useEffect(() => { if (importMutation.error) setError(getErrorMessage(...)) }, [importMutation.error]).

**Recommendation:** Collapse these into a single state machine or reduce the state: (a) derive isConnectionSuccessful = isConnecting && figmaConfig?.connected during render rather than setting it; (b) move the toast.success into the query's onSuccess callback; (c) store only the popup window ref and compute connecting status from whether the ref is non-null; (d) for the mutation error, render error={importMutation.error ? getErrorMessage(importMutation.error) : null} directly instead of duplicating it in local state.

> **Verifier correction:** The scope claim that FigmaImportDialog has three useEffect hooks for OAuth popup detection is slightly overstated — it only has two (focus listener + isConnecting reset); there is no popup-closed polling interval in that file. The figma-configuration-section.tsx has the full three-effect pattern. The error-mirroring effect in FigmaImportDialog is a separate, additional finding that is real. Severity medium is accurate.

### Low-severity notes (not independently verified)
- useEffect fires mutation side-effect in file-explorer (search triggered by state change)

---

## styling — score 6/10

The codebase implements a well-structured Tailwind v4 + shadcn/ui design system with OKLCH tokens in :root/.dark and correct @theme inline bridging. The cn() helper is properly defined and dominates conditional class composition (180 usages). However, four issues break the v4 migration: (1) hsl(var(--)) double-wrapping is scattered across production files and renders invalid CSS in v4, (2) several UI components still carry Tailwind v3 focus-ring patterns (ring-offset-background, focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2), (3) 28 template-literal className constructions bypass twMerge, and (4) repeated arbitrary values (text-[10px] x23, h-[58px] x4, min-h-[80px] x7) should be promoted to tokens or named utilities.

### Strengths
- Correct @theme inline pattern: all semantic tokens are defined in :root/.dark as OKLCH values and bridged to Tailwind via @theme inline { --color-*: var(--*) }, matching the shadcn/ui v4 convention exactly.
- cn() helper is correctly composed as twMerge(clsx(inputs)) in a single lib/utils.ts and is the dominant pattern for conditional class merging (180 call sites found).
- CVA is used for all major multi-variant UI components (Button, Badge, Alert, ToggleGroup, Sidebar menu button) — proper defaultVariants are defined and the result is integrated with cn() for override-friendly composition.
- Dark mode is implemented exclusively via the .dark CSS class applied to the html element by ThemeProvider — no media-query strategy is used, correctly enabling user-controlled toggling.
- Custom git-diff color tokens are correctly scoped in a separate file-explorer.css with :root / .dark overrides using OKLCH format, keeping feature-specific tokens isolated.

### Confirmed findings

#### [high] hsl(var(--)) double-wrapping produces invalid CSS in Tailwind v4

**Files:** `apps/frontend/src/features/tasks/components/minified-event-viewer/minified-last-events-display.tsx:84`, `apps/frontend/src/features/tasks/components/task-card/task-card.tsx:214`, `apps/frontend/src/lib/components/chat/message-history/message-history.tsx:478`, `apps/frontend/src/lib/components/ui/sidebar.tsx:587`, `apps/frontend/src/lib/styles/globals.css:346`

In Tailwind v4, :root tokens are stored as oklch(...) values. Wrapping them in hsl(var(--...)) produces invalid CSS because the resolved value is already an oklch color string — not an HSL triplet. Additionally, the /opacity modifier syntax (hsl(var(--muted) / 0.3)) is a v3-only pattern; in v4, opacity modifiers are handled by the utility class system, not in raw CSS. The sidebar.tsx shadow also hard-codes hsl(var(--sidebar-border)) in an arbitrary shadow value.

**Recommendation:** Replace every hsl(var(--*)) usage with plain var(--*). For opacity, use either a Tailwind utility class (bg-muted/30) or the native CSS alpha syntax oklch(... / 0.3). For the sidebar shadow: use shadow-[0_0_0_1px_var(--sidebar-border)] instead of shadow-[0_0_0_1px_hsl(var(--sidebar-border))]. For the globals.css .json-viewer-clickable rule, use: background-color: oklch(from var(--accent) l c h / 0.2) or a dedicated token.

> **Verifier correction:** The finding is real and affects all cited locations. The globals.css:346 case is oklch double-wrapping (oklch(var(--accent) / 0.2) where --accent already equals oklch(0.97 0 0)) rather than hsl wrapping as the auditor implies, but the invalidity is the same. Severity could be corrected to medium since all affected properties are cosmetic (opacity tints, scrollbar colors, shadow), not functional breakage — browsers silently discard invalid values rather than crash.

#### [high] Tailwind v3 focus-ring pattern used in production UI components

**Files:** `apps/frontend/src/lib/components/ui/textarea.tsx:12`, `apps/frontend/src/lib/components/ui/switch.tsx:12`, `apps/frontend/src/lib/components/ui/toggle-group.tsx:8`, `apps/frontend/src/lib/components/ui/dialog.tsx:62`, `apps/frontend/src/lib/components/ui/sheet.tsx:67`

Five UI components still use the Tailwind v3 focus-ring idiom: ring-offset-background + focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2. In Tailwind v4, ring-offset-background is not a valid utility (ring-offset color is handled differently) and this pattern diverges from the v4 idiom used in the already-migrated Button, Badge, Input, and Select components which use focus-visible:ring-ring/50 focus-visible:ring-[3px]. The inconsistency means some components render a 2px solid ring and others a 3px translucent ring — they do not match visually.

**Recommendation:** Update Textarea, Switch, ToggleGroupItem, the Dialog close button, and the Sheet close button to use the v4 focus pattern already established in Button: focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]. Remove ring-offset-background and ring-offset-2 from these components.

> **Verifier correction:** The finding is real but the severity is inflated. The correct severity is medium, not high. The v3 focus-ring pattern (ring-offset-background + focus-visible:ring-2/ring-ring/ring-offset-2) in textarea.tsx, switch.tsx, and toggle-group.tsx causes visual inconsistency — 2px solid rings vs the 3px translucent rings on Button and Input — but focus rings still render and accessibility is not broken. An additional detail the auditor missed: dialog.tsx:62 and sheet.tsx:67 use focus: (not focus-visible:) which is an extra deviation, making those close buttons show rings on mouse click in addition to keyboard focus, not just the v3-vs-v4 mismatch.

#### [high] Template literal className construction bypasses twMerge — merge conflicts silently ignored

**Files:** `apps/frontend/src/features/navigation/components/sidebar-tasks.tsx:174`, `apps/frontend/src/features/navigation/components/sidebar-tasks.tsx:226`, `apps/frontend/src/features/navigation/components/sidebar-tasks.tsx:238`, `apps/frontend/src/features/commands/components/command-basic-fields.tsx:77`, `apps/frontend/src/features/repositories/components/environment-variables-list.tsx:138`, `apps/frontend/src/features/tasks/components/task-status-icon.tsx:66`, `apps/frontend/src/lib/components/chat/command-autocomplete/command-autocomplete.tsx:68`, `apps/frontend/src/lib/components/chat/file-autocomplete/file-autocomplete.tsx:88`

28 call sites use template-literal string interpolation (className={`base-classes ${conditional}`}) instead of cn(). Without twMerge, if the base string and the conditional string contain conflicting Tailwind utilities in the same property group (e.g., two text-* classes), CSS specificity — not class order — decides which wins, and the result is non-deterministic across browsers. Additionally, any future caller passing an extra className prop will not have it properly merged.

**Recommendation:** Replace all template-literal className constructions with cn(): e.g., className={cn('text-xs truncate leading-snug', isLast ? 'text-sidebar-foreground/80' : 'text-sidebar-foreground/45')}. This is a mechanical refactor — Biome's auto-fix cannot do it, but a codemod is straightforward.

> **Verifier correction:** The finding is real but severity should be low, not high. The template literals bypass cn()/twMerge, but none of the specific instances cited contain classes in the same Tailwind property group, so no CSS merge conflict occurs in practice. The one case with theoretical risk (task-status-icon.tsx:66 combining a text-color class with an external className prop) has no callers that currently trigger it. This is a code consistency/hygiene issue — the codebase has cn() available and should use it — but there are no observable bugs. Severity should be downgraded to low (style/consistency).

#### [high] Dynamic arbitrary-value class construction bypasses Tailwind's static scanner — classes will be purged in production

**Files:** `apps/frontend/src/features/tasks/components/create-task/create-code-task.tsx:496-503`

The getChatInputClassName() function builds class strings like `max-h-[${maxHeight}]` and `min-h-[${minHeight}]` by interpolating external string values into Tailwind's bracket syntax. Tailwind v4's content scanner is static — it cannot see dynamically assembled class strings. Any maxHeight/minHeight value that does not appear literally in source code will be silently purged in production builds, causing layout regressions that only manifest after a production build.

**Recommendation:** Define a fixed set of allowed height variants as a lookup table and select from it: const heightMap = { sm: 'max-h-[200px]', md: 'max-h-[400px]' } as const; return heightMap[size]. If the values come from user configuration, use inline style={{ maxHeight }} instead of a Tailwind class. Never interpolate external strings into Tailwind bracket syntax.

> **Verifier correction:** The finding is real and correctly identified, but the severity assessment should note that minHeight="80px" → min-h-[80px] is accidentally preserved in production because that exact class appears as a static literal elsewhere (e.g., apps/frontend/src/lib/components/ui/textarea.tsx). The genuine production regression is limited to maxHeight values: max-h-[300px] (default and dashboard uses) and max-h-[500px] (dialog use) will be purged, causing the chat textarea to grow without an upper bound. The fix is to replace the dynamic construction with a lookup map of pre-approved literal class strings (e.g., { '300px': 'max-h-[300px]', '500px': 'max-h-[500px]', '80px': 'min-h-[80px]' }) so Tailwind's scanner sees the complete class names as static strings.

#### [medium] Repeated arbitrary values (text-[10px], h-[58px], min-h-[80px]) should be promoted to named tokens

**Files:** `apps/frontend/src/features/tasks/components/task-card/task-card.tsx:194,202,226,423`, `apps/frontend/src/features/tasks/components/task-page.tsx:308`, `apps/frontend/src/features/tasks/components/workspace-header.tsx:30`, `apps/frontend/src/features/tasks/components/task-header/task-left-header.tsx:46`, `apps/frontend/src/features/tasks/components/task-header/task-kiosk-header.tsx:28`, `apps/frontend/src/lib/components/ui/textarea.tsx:12`, `apps/frontend/src/lib/components/chat/message-history/tool-calls/codex-tool-call.tsx:165`

text-[10px] appears 23 times across the codebase, h-[58px] appears 4 times (across the task header/page architecture defining the header height), and min-h-[80px] appears 7 times. These are design decisions masquerading as one-off values. The h-[58px] case is particularly impactful — it is the shared toolbar height and changing it requires a grep-and-replace across 4 files. Promoting to @theme tokens would make design system changes atomic.

**Recommendation:** Add named tokens to @theme inline in globals.css: --spacing-toolbar: 3.625rem (58px); then use h-[--toolbar-height] or a custom spacing token. Add --text-2xs: 0.625rem (10px) for the micro-label size. For min-h-[80px] used as a content minimum, add --spacing-card-content-min. The Textarea's min-h-[80px] is already defined in the shadcn component default and should remain there, but the downstream overrides can reference a token.

#### [medium] Hardcoded semantic colors (text-red-500, text-green-500, bg-white, etc.) without dark-mode pairing in business-logic components

**Files:** `apps/frontend/src/features/tasks/components/task-status-icon.tsx:43,51,55,60`, `apps/frontend/src/features/preview/components/publish-button.tsx:21,64`, `apps/frontend/src/features/sandbox/components/sandbox-overview.tsx:213,224,237`, `apps/frontend/src/features/task-connections/components/connection-chip.tsx:46,49,57`, `apps/frontend/src/lib/components/chat/message-history/error-message.tsx:24,26,28,29,33`

Status indicators and semantic UI elements use hardcoded Tailwind palette colors (text-red-500, text-green-500, text-yellow-500, text-orange-500, bg-white, hover:bg-gray-50) rather than design tokens. The project's own color comment in globals.css explicitly says 'This approach eliminates the need for dark: classes'. Yet connection-chip.tsx returns strings like 'text-emerald-600 dark:text-emerald-400', which is the explicit anti-pattern the token system is designed to avoid. bg-white in publish-button.tsx will be completely invisible in dark mode. These semantic status colors (error, success, warning) belong as named tokens (e.g., --color-status-error, --color-status-success) so they adapt automatically.

**Recommendation:** Add semantic status tokens to globals.css: --status-error, --status-success, --status-warning, --status-info in :root and .dark, then expose via @theme inline. Replace text-red-500/dark:text-red-400 pairs with text-[--status-error]. For publish-button.tsx, replace bg-white hover:bg-gray-50 with bg-background hover:bg-muted/50 which are already themed tokens.

> **Verifier correction:** The finding is real but one nuance is overstated: connection-chip.tsx and error-message.tsx are actually less broken than claimed — they DO include `dark:` overrides so they adapt in dark mode, they just use the wrong pattern (palette classes with dark: variants instead of design tokens). The truly broken cases with NO dark-mode handling are task-status-icon.tsx (the environment-status branch), sandbox-overview.tsx status icons, and publish-button.tsx's bg-white. The auditor frames connection-chip.tsx as "the explicit anti-pattern" when it is actually the least severe instance in the list. Additionally, `text-status-*` classes used in task-status.tsx (the normal status path, not the environment override branch) reference tokens that do not exist in globals.css at all, which is an additional unreported issue beyond what the auditor described.

#### [medium] Custom CSS classes in globals.css declared outside any @layer — they beat utility classes in cascade

**Files:** `apps/frontend/src/lib/styles/globals.css:205-353`

The scrollbar rules, .overflow-auto/.overflow-y-auto hover selectors, [data-streamdown=...] rules, .diff-container, .diff-chunk, .diff-line, .line-numbers, .line-content, and .json-viewer-clickable rules are all declared outside any @layer. In Tailwind v4 the cascade layer order is base < components < utilities. Unlayered CSS beats all of these — meaning Tailwind utility classes cannot override these custom rules. For example, if a component tries to override .diff-line's border-l-4 using a utility class, it will silently fail. The .diff-* classes also use @apply inside non-layered rules, which is an additional anti-pattern.

**Recommendation:** Wrap all custom component-level selectors in @layer components { }. Wrap the scrollbar and data-streamdown rules in @layer base { } since they style native elements. The .diff-* rules should be wrapped in @layer components { } so that utility classes can override them per-instance.

> **Verifier correction:** The finding is real and the severity (medium) is appropriate as-is. No correction needed. The only minor nuance the auditor could clarify is that the `.overflow-auto` / `.overflow-y-auto` hover selectors target `-webkit-scrollbar-thumb` pseudo-elements, not the elements themselves, so they do not compete with Tailwind's `overflow-auto` utility class on property values — the cascade conflict is limited to the `.diff-*` and `.json-viewer-clickable` rules where `@apply`-expanded properties (border, font, bg, etc.) could be blocked if a developer adds conflicting Tailwind utilities to the same element.

### Low-severity notes (not independently verified)
- Massive w-N h-N redundancy — 515 paired square dimensions use verbose syntax instead of size-N shorthand

---

## typescript — score 6/10

The frontend codebase has `strict: true` enabled and Biome enforces `noExplicitAny` as an error for all non-test, non-story files, which eliminates the most dangerous `any` usages. The discriminated union design for events (`Event` type with specific subtypes) and `satisfies` operator usage in event invalidation maps show good TypeScript awareness. However, a cluster of `as unknown as T` double-casts in the repository configuration sections and at the core API boundary (client.ts, tasks.ts) bypass the type system in ways the compiler explicitly rejects — exactly the pattern the project guidelines prohibit. Two key tsconfig flags (`noUncheckedIndexedAccess` and `verbatimModuleSyntax`) are absent, switch statements over the discriminated `Event` union lack exhaustiveness guards, and non-null assertions (`!`) appear in production hooks without being enforced against by Biome in the global rule set.

### Strengths
- strict: true is enabled in tsconfig.json along with noUnusedLocals, noUnusedParameters, and noFallthroughCasesInSwitch, giving a solid strict baseline.
- Biome enforces noExplicitAny as 'error' globally for all non-test, non-story files, preventing raw `any` from accumulating in production code.
- The Event discriminated union is well-designed: eight specific subtypes (AgentEvent, TaskEvent, SandboxEvent, etc.) each extending StreamEvent with a literal `type` field, enabling TypeScript narrowing.
- The EVENT_INVALIDATION_MAP and STATUS_AFFECTING_SUBTYPES constants in event-handlers.ts use `as const satisfies Record<Event['type'], ...>` — the correct pattern for validating shape while keeping the narrowest literal types.
- Biome's `useImportType: error` rule (and `useExportType: error`) enforces `import type` for type-only imports globally, which prevents accidental value imports that inflate bundles.
- The useZodForm hook provides a real Zod-backed `safeParse` boundary for form data, and TanStack Router's `z.object()` search params schema validates URL search parameters at a real type boundary.
- No React.FC usage found anywhere; no default exports found in source files — both project guidelines are followed consistently.

### Confirmed findings

#### [high] Core API client blindly casts every JSON response with 'as unknown as T'

**Files:** `apps/frontend/src/lib/api/client.ts:81`, `apps/frontend/src/lib/api/client.ts:84`, `packages/api-client/custom-instance.ts:50`, `packages/api-client/custom-instance.ts:63`

The legacy `apiClient` (client.ts) and the Orval-generated `customInstance` both cast the raw `response.json()` result straight to the caller-supplied generic `T` without any runtime validation. `unwrapResponse<T>()` (custom-instance.ts:63) is a single-line identity cast `promise as Promise<T>`, making every API call in the app an unchecked cast. If the backend shape changes or the OpenAPI spec drifts, TypeScript will not catch it and the runtime will silently propagate `undefined` into fields assumed to be defined.

**Recommendation:** At the boundary where external data enters the system, use Zod schemas generated from (or aligned with) the OpenAPI spec and call `schema.parse(await response.json())`. At minimum, replace `return (await response.json()) as unknown as T` with a type guard or a narrowed parse step. The `unwrapResponse` helper should be annotated with a comment making its unsafe nature explicit, and callers should be migrated to Zod-parsed equivalents for critical data paths (task data, event payloads).

> **Verifier correction:** The finding is real and the code is accurately described, but severity should be medium rather than high. Unchecked JSON casts are near-universal in TypeScript frontends without explicit schema validation layers; the actual blast radius is limited to cases where the backend OpenAPI spec drifts from the TypeScript types. The `unwrapResponse` cast in custom-instance.ts also has a documented rationale (Orval response-wrapper type mismatch) making it a deliberate trade-off rather than an oversight. A more accurate severity is medium — it represents a missing runtime validation layer, not an active exploitable bug.

#### [high] Repeated 'as unknown as' double-casts in repository config sections signal broken API type alignment

**Files:** `apps/frontend/src/features/repositories/components/sections/code-execution-group.tsx:27`, `apps/frontend/src/features/repositories/components/sections/code-execution-group.tsx:48`, `apps/frontend/src/features/repositories/components/sections/servers-group.tsx:263`, `apps/frontend/src/features/repositories/components/sections/servers-group.tsx:266`, `apps/frontend/src/features/repositories/components/sections/plugins-group.tsx:24`, `apps/frontend/src/features/repositories/components/sections/plugins-group.tsx:45`, `apps/frontend/src/features/repositories/components/sections/preview-environment-group.tsx:24`, `apps/frontend/src/features/repositories/components/sections/preview-environment-group.tsx:38`

Eight `as unknown as` double-casts appear across repository config UI sections. The comments admit the cause: fields like `setupCommands`, `servers`, `plugins`, and `serverStartCommands` are typed in the generated `UpdateRepositoryRequest` as `string[] | null`, `ServerConfig[] | null`, etc. — yet the code comments claim the API types them as `{ [key: string]: unknown } | null`. Checking the actual generated model shows `setupCommands?: string[] | null`, which is already the correct type. The casts exist because the local `Partial<UpdateRepositoryRequest>` form-data type makes the TypeScript compiler complain when the code tries to pass `string[]` where `string[] | null | undefined` is expected — a problem the double-cast silences rather than fixes.

**Recommendation:** Remove the double-casts. The `onUpdateField` callback already uses a properly constrained generic `<K extends keyof UpdateRepositoryRequest>(field: K, value: UpdateRepositoryRequest[K])`. The issue is that `formData` is typed as `Partial<UpdateRepositoryRequest>`, making the fields `T | undefined`, not `T`. Narrow with explicit `?? []` or `?? null` before passing to `updateField` and let TypeScript verify the types correctly: `updateField('setupCommands', e.target.value.split('\n'))` works if `setupCommands: string[] | null` in the request and the split produces `string[]`.

> **Verifier correction:** The issue is real but overstated as "high" severity. The correct severity is low-to-medium. The `as unknown as` double-casts and wrong comments are genuine code quality problems, but they cannot cause runtime errors: casting between `string[]` and `string[] | null | undefined` is semantically safe in both directions. The root cause is not "broken API type alignment" — the actual `UpdateRepositoryRequest` interface already has the correct types (`string[] | null`, `ServerConfig[] | null`). The confusion stems from orphaned generated wrapper type files (`updateRepositoryRequestSetupCommands.ts`, etc.) that declare `{ [key: string]: unknown } | null` but are NOT referenced by the main interface. The developer read these stale files and thought they were authoritative, leading to unnecessary casts and incorrect comments. The fix is: remove the casts, update the comments, and optionally delete or fix the orphaned wrapper type files.

#### [high] SSE event stream: JSON.parse result is blindly cast to Event without validation

**Files:** `apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx:334`, `apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx:341`

The real-time event stream (NDJSON over fetch) parses each line with `JSON.parse(line)` and immediately casts the result `as Event`. No structural validation is performed. If the server sends an event with an unknown `type`, a missing `taskId`, or a malformed `payload`, it propagates through the cache update layer (`applyCacheUpdates`) and subscriber callbacks without TypeScript being able to catch it. Down the call chain, code accesses `event.taskId` and `event.payload` unconditionally.

**Recommendation:** Add a lightweight Zod schema for the base `StreamEvent` shape (type, subType, taskId, eventId, payload) and call `streamEventSchema.safeParse(data)` after JSON.parse. Unknown or malformed events can be discarded or logged. This is the trust boundary where external data enters React state and the query cache — it is the most important place for runtime validation in the entire frontend.

> **Verifier correction:** The finding is genuine and the code is accurately cited. The severity should be adjusted from high to medium-to-high (medium in a fully-controlled deployment, high if considering an attacker-in-the-middle or backend bug scenario). The recommended fix would be to validate the parsed object against the expected shape (e.g., checking that `typeof data === 'object' && data !== null && typeof data.taskId === 'string' && typeof data.type === 'string'`) before casting, or to use a Zod schema for full runtime validation. The finding is not inflated in terms of existence, only slightly in terms of severity given the same-origin trust context.

#### [medium] sendMessage in tasks.ts uses 'as unknown as ApiSendMessageRequest' because the local type adds a UI-only field

**Files:** `apps/frontend/src/features/tasks/api/tasks.ts:88-112`

The `sendMessage` method defines its own `variables` parameter type that extends `SendMessageRequest` with a `fileUploads` UI-only field. After destructuring to remove `fileUploads`, the spread `payload` cannot be directly passed to `sendMessageHandler` because TypeScript can't verify the remaining fields still conform to `ApiSendMessageRequest` — so a double-cast `payload as unknown as ApiSendMessageRequest` is used. The type system is being worked around rather than expressed correctly.

**Recommendation:** Define a proper local type `SendMessageVariables = ApiSendMessageRequest & { fileUploads?: FileUpload[] }` and use `Omit<SendMessageVariables, 'fileUploads'>` as the type of `payload` after destructuring, or create a mapped type that strips `fileUploads`. This would let TypeScript verify that `payload` satisfies `ApiSendMessageRequest` without a cast.

#### [medium] create-task-form.tsx casts Zod-validated data to CreateTaskRequest, defeating the point of Zod validation

**Files:** `apps/frontend/src/features/tasks/components/create-task-form.tsx:107`

After running `validate(taskData)` with Zod `safeParse` on line 96, the validated `result.data` is passed to `createTask.mutate()` via `result.data as unknown as CreateTaskRequest`. The Zod schema (`CreateTaskRequestSchema`) uses `z.string()` for `agentType`, `repositoryId`, etc., but omits fields like `agentMode` and `contexts` that exist on the real `CreateTaskRequest`. The local Zod schema does not exactly match `CreateTaskRequest`, so TypeScript rejects the direct assignment — and a double-cast silences the complaint. This means the type safety benefit of Zod validation is immediately discarded.

**Recommendation:** Align the Zod schema with `CreateTaskRequest` exactly. Use `z.object({ agentMode: z.enum(['edit', 'plan']).optional(), ... })` so that `z.infer<typeof CreateTaskRequestSchema>` is assignable to `CreateTaskRequest` without a cast. Alternatively, pass `result.data` directly to a typed `createTask` API function that accepts the Zod-inferred type.

> **Verifier correction:** The finding is real but the description is partially wrong. AgentMode is not an enum — it is `type AgentMode = string` (a string alias), so z.string() is fully compatible. The actual cause of the incompatibility requiring `as unknown as CreateTaskRequest` is the `contexts` field: `z.array(z.unknown())` infers as `unknown[]` which is not assignable to `MessageContextInput[]`. The fix is to use a properly typed Zod schema for contexts (or z.custom()) rather than z.unknown(). Severity medium is appropriate — the cast defeats type safety specifically for contexts, but runtime behavior is still likely correct because the actual contextItems values flowing through are already of the right shape.

#### [medium] Non-null assertions (!) in TanStack Query hooks bypass the 'enabled' guard at the type level

**Files:** `apps/frontend/src/features/preview/hooks/use-task-events.ts:7`, `apps/frontend/src/features/preview/hooks/use-task-events.ts:8`, `apps/frontend/src/features/files/hooks/use-file-content.ts:13`, `apps/frontend/src/features/task-templates/hooks/use-task-templates.ts:34`, `apps/frontend/src/features/repositories/components/sections/servers-group.tsx:292`

Multiple hooks accept an optional parameter (`taskId: string | undefined`) and use `enabled: !!taskId` to skip the query when the value is absent — which is the correct TanStack Query pattern. However, they then use `taskId!` inside `queryKey` and `queryFn`, asserting non-null at the type level even though `taskId` could actually be `undefined` if `enabled` is misconfigured or if TanStack Query behaviour changes. This is a latent runtime crash risk. The Biome `noNonNullAssertion` rule is disabled only for test files, so these should be caught — but the rule is not explicitly added to the global rules section, relying on `recommended` defaults.

**Recommendation:** Use the `taskId ?? ''` pattern for queryKey (matching the taskTemplates hook which already does `id ?? ''` for the key) and wrap the queryFn: `queryFn: () => taskId ? fetchTaskEvents(taskId) : Promise.reject(new Error('taskId required'))`. This eliminates the non-null assertion and makes the intent explicit. Enable `style.noNonNullAssertion: 'error'` explicitly in the global Biome rules rather than relying on the recommended set, so future assertions are caught at lint time.

> **Verifier correction:** The finding is real but the severity is inflated. The non-null assertions exist in the cited locations (except servers-group.tsx:292 which is unrelated — it correctly uses `!` after a `.filter()` guard). However, there is no actual runtime crash risk: (1) TanStack Query never invokes the queryFn when enabled:false, making the `taskId!` in queryFn dead code in the disabled path; (2) the `!` operator has no runtime effect — `undefined!` is still `undefined`. The Biome rule `noNonNullAssertion` is indeed not enabled globally (confirmed in biome.json). This should be severity: low/informational — a code clarity/style issue where `taskId ?? ''` or a conditional type would be cleaner, not a crash risk.

### Refuted claims (checked and dismissed)

- **Switch statements over the Event discriminated union have no exhaustiveness check** — The claim contains one fully incorrect finding and two real-but-intentional omissions that are mischaracterized.

1. task-status.tsx:68 — the function `getTaskStatusConfig(status: string)` does NOT switch over the `Event` discriminated union. It switches over a plain `string` parameter that represents task lifecycle status (`open`, `running`, `error`, `merged`, `closed`, etc.). The file does not even import the `Event` type. The switch already has a `default:` branch returning a fallback value, 

### Low-severity notes (not independently verified)
- tsconfig.json missing noUncheckedIndexedAccess and verbatimModuleSyntax
- localStorage reads are cast without validation (theme-provider, use-local-storage)

---

## data-fetching — score 7/10

The frontend follows TanStack Query discipline well in its core task domain: centralized hierarchical query keys, no useEffect-based fetching, clean mutation patterns with optimistic updates, proper SSE integration via a singleton provider, and a well-configured QueryClient with 5-minute staleTime and explicit gcTime. The main recurring issues are a small cluster of inline ad-hoc query key strings that bypass the centralized registry (primarily for auth and OAuth), the absence of the queryOptions() factory pattern which means getQueryData requires manual generic type parameters throughout, and a handful of useEffect-to-useState bridges that copy server state into local state for form editing. The codebase scores high for overall discipline but has several medium-severity gaps that could cause subtle cache-miss bugs or stale data in the settings and profile sections.

### Strengths
- Global QueryClient is configured with staleTime: 1000*60*5 (5 min) and gcTime: 1000*60*10 (10 min), disabling refetchOnWindowFocus globally — avoids the default refetch-on-mount storm.
- The centralized queryKeys object in src/lib/api/query-keys.ts is hierarchical and well-structured: tasks.all → tasks.lists() → tasks.list(filters) → tasks.detail(id) → tasks.messages(id) cascades cleanly for invalidation.
- SSE integration is architecturally correct: a single GlobalEventStreamProvider opens the streaming fetch connection outside React's query lifecycle and pushes updates into the cache via setQueryData and invalidateQueries — no EventSource inside a queryFn.
- The useSendMessage optimistic mutation correctly follows the 4-step pattern: cancelQueries (both messages and detail), snapshot getQueryData, setQueryData optimistically, and rolls back in onError.
- useTaskActivity uses useState + subscribeToAll (event subscription) for pure client-side derived state, correctly never storing SSE data in TanStack Query for ephemeral display-only activity — an appropriate design choice.
- The useInfiniteQuery for task messages uses cursor-based pagination (nextBeforeId) rather than offset pagination, avoiding the duplicate/gap problem on live data.
- The refreshNewestMessagesPage helper replaces only the newest page in the infinite query cache rather than invalidating the whole query — this preserves scroll position, a sophisticated optimization.

### Confirmed findings

#### [high] Inline ad-hoc query key strings bypass the centralized registry

**Files:** `apps/frontend/src/features/auth/hooks/use-auth.ts:21`, `apps/frontend/src/features/settings/hooks/use-github-connection.ts:60,81,102,120`, `apps/frontend/src/features/settings/hooks/use-profile.ts:11`, `apps/frontend/src/routes/admin.tsx:12`, `apps/frontend/src/features/figma/hooks/use-figma-import.ts:28`, `apps/frontend/src/features/settings/hooks/use-figma.ts:28`, `apps/frontend/src/features/settings/hooks/use-github-connection.ts:32`

Six distinct locations define query keys as bare inline string arrays instead of using queryKeys from src/lib/api/query-keys.ts. The auth session key ['auth', 'session'] is used in useAuth (as the query), in admin.tsx (as ensureQueryData), in use-github-connection.ts (invalidateQueries x4), and in use-profile.ts (invalidateQueries). The Figma OAuth URL uses ['figma-oauth-url'] and GitHub OAuth uses ['github-oauth-url'] — neither is in query-keys.ts. The figma imports hook uses ['figma', 'imports', importId]. Because these are not in the central registry, there is no single place to refactor them, they cannot be invalidated hierarchically, and any typo silently misses the cache.

**Recommendation:** Add auth: { session: () => ['auth', 'session'] as const } to query-keys.ts and add entries for figmaOAuthUrl, githubOAuthUrl, and figmaImports. Replace every ['auth', 'session'] literal with queryKeys.auth.session(). For the OAuth URL queries, since they intentionally disable caching (staleTime: 0, gcTime: 0), keeping them in the registry does not change behavior but preserves refactoring safety.

> **Verifier correction:** The finding is real and the core issue is accurately described, but the severity is slightly inflated for two of the six violations. The OAuth URL keys (['figma-oauth-url'] in use-figma.ts:28, ['github-oauth-url'] in use-github-connection.ts:32) both have gcTime: 0 and staleTime: 0 with comments explicitly stating they are single-use and intentionally not cached. For those two, the practical risk of a missed cache invalidation is near-zero. The true high-severity subset is: the ['auth', 'session'] key spread across 6 locations (use-auth.ts:21, admin.tsx:12, use-github-connection.ts:60/81/102/120, use-profile.ts:11) with no registry entry, and ['figma', 'imports', importId] in use-figma-import.ts:28. The finding should rate the auth session cluster as high severity, the figma imports key as medium, and the OAuth URL keys as low (convention violation only, no cache coherence risk).

#### [medium] queryOptions() factory not used — manual generic type parameters required on getQueryData throughout

**Files:** `apps/frontend/src/features/tasks/hooks/use-tasks.ts:158,327,330,334,447,554,664`, `apps/frontend/src/features/tasks/utils/query-helpers.ts:162,212`

None of the query hooks use TanStack Query v5's queryOptions() factory. As a result, every call to queryClient.getQueryData must supply an explicit generic type parameter (e.g., getQueryData<TaskResponse>(...)) because TypeScript cannot infer the return type from a raw queryKey array. With queryOptions(), the DataTag on the key infers the type automatically, making type parameters unnecessary and making typos in option names (e.g. stallTime) a compile error rather than a silent no-op. There are at least 9 explicit getQueryData<T>() calls spread across two files.

**Recommendation:** Refactor query factories to use queryOptions(): e.g., export const taskDetailOptions = (taskId: string) => queryOptions({ queryKey: queryKeys.tasks.detail(taskId), queryFn: () => tasksApi.getTask(taskId), staleTime: 30_000 }). Pass the same options object to useQuery and queryClient.getQueryData/setQueryData. The explicit generic parameters on all getQueryData calls can then be removed.

> **Verifier correction:** The finding is real and the code evidence is accurate. However, severity should be low rather than medium. The existing pattern of explicit generic type parameters on `getQueryData<T>()` is functionally equivalent to what `queryOptions()` would provide — both result in correctly typed return values enforced by TypeScript. The main benefits of migrating to `queryOptions()` are ergonomic: removing boilerplate type annotations and catching typos in query option names (e.g., `stallTime` vs `staleTime`) at compile time. These are refactoring quality improvements, not correctness defects. There are no runtime consequences to the current pattern and no risk of data being fetched with the wrong type in practice.

#### [high] Server state copied into useState via useEffect in profile and form dialogs

**Files:** `apps/frontend/src/features/settings/components/profile-configuration-section.tsx:20-28`, `apps/frontend/src/features/tasks/components/task-header/edit-task-dialog.tsx:48-58`

ProfileConfigurationSection copies server state (user.name, user.email from useAuth()) into useState via useEffect. If the auth query background-refreshes, the local state will not update to reflect the new values. The same pattern exists in EditTaskDialog which copies currentName and currentDescription props (derived from server state) into local useState fields via a useEffect that watches isOpen. While the dialog pattern is more defensible (controlled form input), it still means the form will silently show stale values if the task is renamed by another user while the dialog is closed and then re-opened without a refetch.

**Recommendation:** For ProfileConfigurationSection, initialize the useState with a lazy initializer using the current user value, and rely on the mutation's onSuccess to invalidate the auth session query — the component will re-render with fresh data naturally. For the form dialogs, the current approach is acceptable for short-lived transient UI, but the useEffect dependency should be tightened; consider using a key prop on the dialog (e.g., key={isOpen ? currentName : 'closed'}) to remount and reinitialize state instead of the effect.

> **Verifier correction:** The issue is real but severity should be reduced from high to medium. The ProfileConfigurationSection finding is legitimate and material: useEffect watching `[user]` will fire on background refetches, resetting any unsaved form edits the user has typed. The EditTaskDialog finding is considerably weaker — props-based form initialization that resets on dialog open is a defensible and common React pattern, and the dependency array watches both `isOpen` and the prop values, not just `isOpen` as the auditor claimed (line 59: `[isOpen, currentName, currentDescription]`). The stale-values risk for EditTaskDialog is a theoretical edge case dependent on the parent's cache invalidation strategy, not a structural defect in the dialog itself.

#### [medium] EventSource used inside a useEffect hook (server logs SSE stream bypasses query cache)

**Files:** `apps/frontend/src/features/preview/hooks/use-server-logs-stream.ts:27-73`

useServerLogsStream creates a new EventSource inside a useEffect and accumulates log lines into a local useState string. This is entirely outside TanStack Query: there is no cache, no deduplication, no background reconnect managed by the query layer, and no way to share the connection if multiple components mount this hook simultaneously. The log data also exists only in component state, so it resets on every remount. Additionally, the onerror handler silently swallows all errors (only marks isConnected: false) with no retry count, error message, or user-visible feedback — diverging from the project's stated error-handling guidelines.

**Recommendation:** Consider two alternatives: (1) If logs are short-lived and only needed in one place, keep the EventSource approach but add an error state with the last error message, a retry counter, and expose error to the caller. (2) If logs need to be shared or persisted across remounts, push received log lines into the query cache via queryClient.setQueryData(queryKeys.tasks.serverLogs(taskId, serverName), ...) from outside the hook, giving the log data the same cache benefits as other server state.

> **Verifier correction:** The finding is accurate and the severity of "medium" is appropriate. One nuance the auditor overstated: the onerror comment ("EventSource automatically reconnects on transient errors") is true per the EventSource spec, so not setting an error on onerror is defensible for transient failures. However, since error state is never populated at all (even on permanent failures where readyState reaches CLOSED), the user truly gets no error feedback for unrecoverable connection failures — the error branch in server-logs-viewer.tsx line 39 (`if (error && !logs)`) is effectively dead code. The bug is narrower than "all errors silently ignored" — it's specifically that permanent/unrecoverable errors produce no user feedback.

### Refuted claims (checked and dismissed)

- **useSendMessage has no onSettled invalidation — relies solely on SSE for eventual consistency** — The code at lines 382-397 does exactly what the auditor describes — no onSettled, and a comment saying SSE events will update the cache. However, the framing of this as a correctness gap is inaccurate for several reasons:

1. SSE reconciliation is complete and correct: when SSE delivers a `taskTurn.completed`, `agent.session_init`, or `agent.message` event, `invalidateMessagesForEvent` (event-handlers.ts:181) calls `refreshNewestMessagesPage` (use-tasks.ts:491), which fetches the newest page fro

### Low-severity notes (not independently verified)
- feedback query keys are not rooted under the main swarm hierarchy
- queryKeys.tasks.all is a raw array constant, not a function — inconsistent with rest of the hierarchy
- select option underutilized — derived data computed in useMemo in components instead
