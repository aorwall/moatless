# Task Page Restructure: Separate Layout Shells from Content Panels

Status: proposal (research + suggested structure, no implementation yet)
Date: 2026-07-11

## Goal

Make the task page composable so that mobile, desktop, and kiosk experiences are
**separate layout compositions** built from the **same shared content panels**
(sidebar, chat, preview, files, sandbox controls), instead of one component tree
full of `isMobile` / `kiosk` if-clauses.

## Current state (what the research found)

### The task page today

```
routes/tasks/$task-id/index.tsx (TaskDetail)
  └─ TaskProviders (TaskDataProvider → TaskUIProvider)
       └─ TaskPage (features/tasks/components/task-page.tsx, ~430 lines)
            └─ TaskPageLayout (branches on isMobile AND kiosk internally)
                 ├─ header:  TaskLeftHeader | TaskKioskHeader
                 ├─ chat:    ChatContainer
                 └─ workspacePanel: Preview | LazyFileExplorer | SandboxWorkspacePanel
```

### Mixed concerns identified

1. **`TaskPageLayout` is three layouts in one file** (`task-page-layout.tsx`):
   an `if (isMobile)` early-return (line 101), a desktop resizable split, and a
   `kiosk` flag that swaps panel order and the expand-button variant
   (lines 142–175). It also owns `ChatToggleContext`, which `TaskKioskHeader`
   consumes — a content component hard-coupled to one specific layout.

2. **`TaskPage` is an orchestrator doing five jobs**: task/sandbox queries,
   SSE event accumulation (`sandboxEvents`, `startupEvents`, dedup ring,
   lines 104–177), chat context-item state, breakpoint handling, and — most
   tellingly — **three ref-callback escape hatches** that bridge panels which
   have no way to talk to each other:
   - `chatToggleRef` (TaskPageLayout → TaskPage → Preview's `TOGGLE_CHAT` postMessage)
   - `previewNavigateRef` (Preview → TaskPage → ChatContainer's URL clicks)
   - `commandExecutorRef` (ChatContainer → TaskPage → Preview's troubleshoot button)

3. **Leaf components make viewport decisions.** `WorkspaceHeader`
   (`workspace-header.tsx:27`), `PreviewHeader` (`preview-header.tsx:67`), and
   `TaskLeftHeader` (`task-left-header.tsx:22`) each call `useIsMobile()` /
   read `isMobile` internally to hide `ServerTabs`, `ViewportToggle`, or show
   `MobileViewToggle`. A reusable header should not know what device it's on.

4. **`useTaskView` leaks the breakpoint.** A hook whose job is URL view state
   also returns `isMobile` (`use-task-view.ts:89`), so callers pick up
   viewport awareness just by needing the view.

5. **Content panels embed layout primitives.** `Preview` hard-codes a vertical
   `ResizablePanelGroup` for its console (`preview.tsx:640–794`, with a
   duplicated collapsed/expanded branch), and `SandboxWorkspacePanel` embeds
   another one for controls vs. logs.

6. **Event state is lifted and drilled.** `sandboxEvents` is accumulated in
   `TaskPage` and prop-drilled into both `Preview` and `SandboxWorkspacePanel`.

### What is already in good shape (foundations to build on)

- **View/navigation state already lives in the URL** (`?view=`, `?path=`,
  `?file=`, `?viewport=` via `use-task-view.ts`). URL state survives a
  mobile↔desktop tree swap for free — this is the recommended source of truth.
- **Most panels are already self-contained**: `ChatContainer`,
  `TaskControlPanel`, `LazyFileExplorer`, `TaskLeftHeader`, `PreviewHeader`
  fetch their own data via hooks (`useTaskId()` + TanStack Query, which
  dedupes across callers). Server state survives tree swaps via the query cache.
- **`TaskDataProvider` / `TaskUIProvider`** already exist as task-scoped
  context roots above the layout — the natural home for state that must
  outlive a layout swap.

## Best practices (external research synthesis)

> Note: live web access was unavailable in this environment; this synthesis is
> from training-data knowledge (through mid-2025) of TanStack Router, Next.js
> App Router, shadcn/ui, Radix, and widely-cited React architecture writing.

1. **Layout shells are pure slot-accepting components.** A shell takes
   `ReactNode` slots (or compound children) and only arranges them; it never
   imports domain components or fetches data. (Next.js `layout.tsx`, TanStack
   Router layout routes, Radix `Slot` all converge here.)
2. **CSS-first responsiveness for visual reflow; JS branching only when the
   interaction model differs.** Chat-beside-preview vs. one-panel-at-a-time is
   a genuinely different interaction model → separate shells chosen by one
   `matchMedia` check is the right tool (this is also shadcn/ui's documented
   `Dialog`-on-desktop / `Drawer`-on-mobile pattern).
3. **When trees swap at a breakpoint, all important state must live outside
   both trees**: URL params, query cache, or a context/store above the branch
   point. Never local `useState` inside a component that unmounts on resize.
4. **URL search params are the primary cross-panel coordination channel**
   (selected view, panel visibility, preview path) — SSR-safe, deep-linkable,
   survives swaps.
5. **For imperative cross-panel actions, use a scoped context (or store) at
   the feature root** — not ref callbacks threaded through a parent, and not
   global event emitters.
6. **Compound/headless page-level primitives**: a root that owns the
   breakpoint decision with children that declare *what they are*
   (sidebar/main/panel), not *where they go*, keeps `isMobile` out of every
   consumer.

## How comparable products do this (verified against their repos)

**LobeChat** ([lobehub/lobe-chat](https://github.com/lobehub/lobe-chat), ~65k
stars, chat/agent product — the closest analog to our ask). Verified from the
repo tree (2026-07):

- `src/routes/(main)/` and `src/routes/(mobile)/` are **separate route trees
  per device class** (plus `(desktop)` for their Electron app and `(popup)`).
  The entry dispatches to the desktop or mobile router based on device type —
  the same "one branch point, then separate compositions" shape as our
  `task-page.tsx` switch, just hoisted to the router.
- Each route owns a `_layout/` folder containing pure arrangement components
  (`routes/(main)/agent/_layout/Sidebar/`, `_layout/HeaderSlot.tsx`;
  `routes/(mobile)/chat/_layout/`). Mobile routes also carry mobile-only glue
  (`(mobile)/chat/features/ChatHeader`, `Topic`).
- Both trees compose **shared domain components from `src/features/`**
  (112 top-level entries: `Conversation`, `ChatInput`, `AgentSetting`, …).
  Layout per device, content shared — exactly the target of this proposal.

**bolt.diy** ([stackblitz-labs/bolt.diy](https://github.com/stackblitz-labs/bolt.diy),
open-source Bolt: chat left, code/preview workbench right — same page shape as
our task page): `components/chat/` and `components/workbench/`
(Workbench, Preview, DiffView) stay separate, and cross-panel coordination
goes through **central stores** (nanostores: `workbench.ts` is the
coordinator and owns *chat visibility*, plus `previews.ts`, `editor.ts`,
`files.ts`). Notably, "toggle the chat panel" living in a store rather than
in ref callbacks is precisely our `TaskPanelControls` move.

**OpenHands** ([OpenHands frontend](https://github.com/OpenHands/OpenHands/blob/main/frontend/README.md),
agent chat + terminal/editor panels): feature-based component organization, a
separate reusable UI library (`openhands-ui/`), Zustand for cross-panel UI
state, TanStack Query for server state — same state split we already have.

**Sentry** ([getsentry/sentry `static/app/`](https://github.com/getsentry/sentry/tree/master/static/app),
verified): a dedicated `views/` directory (page-level composition) next to
`components/` (shared) — a "composition layer" under a different name, in
production at very large scale for a decade.

The through-line: every one of these separates *arrangement* code from
*content* code structurally (LobeChat `_layout/` vs `features/`, bolt.diy
workbench vs chat, Sentry `views/` vs `components/`), and the two that have
real mobile experiences (LobeChat, and shadcn/ui's documented
Drawer-on-mobile/Dialog-on-desktop pattern) branch **once** at a high level
into separate compositions rather than sprinkling `isMobile` through shared
components. Cross-panel actions consistently live in a scoped store/context,
not in parent-mediated callbacks.

## Terminology (audit + glossary)

An audit of the current names found the same word carrying multiple meanings,
and the same concept carrying multiple names. Verified collisions:

- **"view" means three things**: the `?view=` URL param (which workspace
  content is shown), `ChatView` (a dumb scroll wrapper in
  `lib/components/chat/chat-view.tsx`), and — in earlier drafts of this plan —
  the layout shells. Resolved below by reserving *view* for the URL-param
  meaning and calling the shells *layouts* (which also matches the existing
  `TaskPageLayout` lineage and LobeChat's `_layout/` convention).
- **"viewport"/"mobile" mean two things**: the user's device class
  (`useIsMobile`, `MobileViewToggle`) *and* the simulated screen size inside
  the preview iframe (`ViewportMode = 'mobile' | 'tablet' | 'desktop'`,
  `ViewportToggle`, `?viewport=`). "Mobile viewport on a desktop device" is
  currently expressible and ambiguous.
- **"environment" vs "sandbox"**: the API DTO says `environmentStatus` /
  `environmentError` (86 uses) while everything else — hooks, components,
  backend, UI labels — says sandbox (`useSandboxStatus`, `sandboxStatus`,
  `SandboxStatusOverlay`, the "Sandbox" tab, 75 uses).
- **`ServerTabs` isn't only servers**: it renders Files and Sandbox tabs too
  (its own comment in `workspace-header.tsx:32` explains this away).
- **`SandboxEvent` isn't only sandbox events**: it carries `server` and
  `taskTurn` events as well.
- **`TaskDetail` vs `TaskPage`**: the route component and the page component
  are two names for the same page.
- **`TaskLeftHeader`**: named for its desktop position; on mobile there is no
  "left".

### Glossary (one word, one meaning)

| Term | Meaning | Appears as |
|---|---|---|
| **task page** | the whole `/tasks/$task-id` experience | `TaskPage`, `layouts/task-page.tsx` |
| **layout** | an arrangement shell for one experience; owns splits/toggles, no data | `DesktopTaskLayout`, `MobileTaskLayout`, `KioskTaskLayout` |
| **panel** | a content region a layout arranges | `ChatPanel`, `WorkspacePanel`, `FilesPanel`, `SandboxPanel`, `PreviewPanel`, `LogsPanel` |
| **view** | the user-selectable workspace content; URL `?view=` | `useTaskView`, `view-registry`, `ViewTabs`, `ViewSwitcher` |
| **workspace** | the non-chat side of the task page | `WorkspacePanel`, `WorkspaceHeader` |
| **sandbox** | the task's runtime (pod + lifecycle) | `useSandboxStatus`, `SandboxStatusOverlay`, sandbox view |
| **server** | a process in the sandbox exposing a port | `ServerEntry`, `ServerStatusView`, server views |
| **preview** | iframe rendering of a running server | `Preview`, `previewPath`, `previewUrl` context |
| **device (frame)** | simulated screen size inside the preview | `DeviceFrame`, *renamed* device toggle/mode |
| **mobile / desktop** | the user's device class (breakpoint) — used **only** inside `layouts/` | `useIsMobile`, `MobileTaskLayout` |
| **kiosk** | the chrome-less display variant | `KioskTaskLayout`, kiosk route |

### Renames

| Current | New | Why |
|---|---|---|
| `TaskDetail` (route component) | `TaskPageRoute` (or inline) | One name for the page: task page |
| `TaskPageLayout` | *(deleted)* → `DesktopTaskLayout` / `MobileTaskLayout` / `KioskTaskLayout` | One layout per experience |
| `MobileViewToggle` | `ViewSwitcher` | It switches views; "mobile" is where it's *placed*, not what it *is* |
| `ServerTabs` | `ViewTabs` | Contains Files + Sandbox tabs, not just servers |
| `SANDBOX_VIEW` (in `server-tabs.tsx`) | `types/view-registry.ts` with `SANDBOX_VIEW`, `FILES_VIEW`, `CHAT_VIEW` | Views defined in one place, not in a component file |
| `ChatContainer` | `ChatPanel` (absorbed into `panels/chat-panel.tsx`) | Aligns with the panel vocabulary; drops the vague "Container" |
| `ChatView` (lib wrapper) | `ChatScrollArea` | Frees "view" for the `?view=` meaning |
| `SandboxEvent` / `useHistoricalSandboxEvents` | `LifecycleEvent` / `useHistoricalLifecycleEvents` (in `features/events/`) | It also carries server + turn events |
| `ViewportMode` / `ViewportToggle` / `usePreviewViewport` | `DeviceMode` / `DeviceToggle` / `usePreviewDevice` (URL param `viewport` → `device`) | Frees "mobile/desktop/viewport" for device class; matches the existing `DeviceFrame` component |
| `TaskLeftHeader` | `TaskHeader` | Position-free; it's *the* task header, layouts decide where it goes |
| `workspaceContent` / `workPanel` / `workspacePanel` (locals) | `WorkspacePanel` | One name for the region |
| `environmentStatus` / `environmentError` (API DTO) | *keep* — API contract change is out of scope | But: no **new** frontend identifiers use "environment"; UI vocabulary is sandbox. Longer-term, align the DTO to `sandboxStatus` in a coordinated backend change |

Rules going forward: "mobile"/"desktop"/"kiosk" may appear only in `layouts/`
filenames and `useIsMobile`; "view" always means the `?view=` selection;
"panel" names things layouts arrange; positional words (left/right) never
appear in component names.

## Proposed structure

### Three-layer model

| Layer | Responsibility | Allowed to know about |
|---|---|---|
| **Providers** (task-scoped) | Data, events, cross-panel actions, composer state | TanStack Query, SSE, URL |
| **Layouts** (one per experience) | Arrangement only: splits, drawers, toggles, which header variant, panel sizes | Layout primitives, `useIsMobile` (only at the single entry point) |
| **Panels** (shared content) | Domain UI; fetch own data via hooks | Providers' hooks. **Never** `useIsMobile`, never `ResizablePanel` |

### Directory structure

The task page composes several features (preview, files, events), and that
composition lives in `features/tasks`. That in itself is fine — the problem
is that the dependency is currently **bidirectional** (verified by import
analysis):

- `features/tasks` → `features/preview`: `Preview`, `SandboxStatusOverlay`,
  `SANDBOX_VIEW` (a constant exported from a component file,
  `server-tabs.tsx`), `useServerLogsStream`, `useHistoricalSandboxEvents`
- `features/preview` → `features/tasks`: `useTaskView`, `useTaskId`,
  `useTaskDetailQuery`, `useSandboxRetry`, `useDeployTask`,
  `WorkspaceHeader`, `ServerLogsViewer`
- `features/files` → `features/tasks`: `useSelectedFile`,
  `useTaskSandboxStatusQuery`
- `lib/components/chat` (MessageHistory, ChatInput, context-items) is domain
  chat UI consumed only by the tasks feature — it lives in `lib` but isn't
  generic.

**Decision: the new structure lives inside `features/tasks/`.** The tasks
feature becomes the app's *composition feature* — the one place allowed to
import from other features. The established rule this satisfies (per
[bulletproof-react](https://github.com/alan2207/bulletproof-react/blob/master/docs/project-structure.md):
*"compose different features at the application level"*, unidirectional
imports) is expressed as:

- `features/tasks` may import `preview`, `files`, `events`, `figma`, etc.
- No other feature may import `features/tasks` (this is the rule that kills
  the current cycle — today preview and files both reach back into tasks).
- `features/events` is treated as foundational (like `lib`): any feature may
  import it, since SSE/event types are cross-cutting.
- No feature imports another feature except through these two rules.
- Enforced mechanically with Biome `noRestrictedImports` per-directory
  overrides (the Biome equivalent of bulletproof-react's
  `import/no-restricted-paths`).

### Target structure inside `features/tasks/`

New top-level folders `layouts/` and `panels/` make the three-layer model
visible in the directory tree; `contexts/` grows the new providers:

```
features/tasks/
├── layouts/                             # LAYOUT LAYER — arrangement only
│   ├── task-page.tsx                  # the single useIsMobile() switch (moved from components/)
│   ├── desktop-task-layout.tsx          # resizable chat|workspace split + desktop header slots
│   ├── mobile-task-layout.tsx           # single panel + ViewSwitcher + mobile header slots
│   ├── kiosk-task-layout.tsx            # workspace-first split, kiosk header, no console
│   └── view-registry.ts               # SANDBOX_VIEW etc. (moved out of preview/server-tabs.tsx)
│
├── panels/                            # CONTENT LAYER — cross-feature panel wrappers
│   ├── chat-panel.tsx                 # ChatContainer + error boundary
│   ├── workspace-panel.tsx            # view→panel switch (files/sandbox/server), NO layout
│   ├── files-panel.tsx                # features/files explorer + readiness overlay
│   ├── sandbox-panel.tsx              # TaskControlPanel + LogsPanel (split owned by views)
│   └── preview-panel.tsx              # features/preview iframe/overlays (console owned by views)
│
├── contexts/                          # STATE LAYER — task-scoped providers
│   ├── task-providers.tsx             # existing entry, composes the stack
│   ├── task-data-provider.tsx         # existing
│   ├── sandbox-events-provider.tsx    # NEW: event accumulation moved out of TaskPage
│   ├── task-composer-provider.tsx     # NEW: contextItems + composer-adjacent state
│   └── task-panel-controls.tsx        # NEW: semantic actions; impls provided per view
│
├── components/                        # domain building blocks (purified, no layout/breakpoints)
│   ├── chat/                          # MOVED here from lib/components/chat + chat-view/*
│   ├── headers/                       # task-left-header, task-kiosk-header, workspace-header
│   └── ...                            # control panel, dashboards, existing components
│
├── hooks/                             # use-task-view (isMobile removed), queries, etc.
├── api/ · types/                      # existing
```

Deleted: `components/task-page.tsx` (replaced by `layouts/`),
`components/task-page-layout.tsx`, `ChatToggleContext`,
`components/mobile-view-toggle.tsx` (renamed → `ViewSwitcher`, likely
living next to the headers).

Outside `features/tasks`, only two kinds of moves:

```
lib/
├── components/ui/                     # unchanged (shadcn primitives)
└── layout/                            # NEW generic primitives extracted from content:
                                       #   collapsible-split, stacked-panel, header-frame
features/events/                       # gains sandbox-event mapping + SandboxEvent types
                                       #   (moved from features/preview/hooks + tasks/task-page)
```

### Specific moves

| What | From | To | Why |
|---|---|---|---|
| `MessageHistory`, `ChatInput`, context-items | `lib/components/chat/` | `features/tasks/components/chat/` | Domain UI consumed only by tasks; `lib` should stay generic |
| `TaskPage`, view shells | `components/task-page*.tsx` | `features/tasks/layouts/` | Makes the layout layer visible; one file per experience |
| `SANDBOX_VIEW` constant | `preview/components/server-tabs.tsx` | `features/tasks/types/view-registry.ts` | It names a task-page view; constants don't belong in component files. Removes a tasks→preview deep import |
| `useHistoricalSandboxEvents`, `SandboxEvent`/`StartupEvent` types, `toSandboxEvent` | `features/preview/` + `task-page.tsx` | `features/events/` | Event-domain code; lets preview and tasks both use it without importing each other |
| Console/controls resizable splits | inside `Preview` / `SandboxWorkspacePanel` | `lib/layout/` primitives, composed by `layouts/` | Layout out of content |
| `WorkspaceHeader` | `features/tasks/components/` | `features/tasks/components/headers/` (purified to slots) | Stops preview importing it; preview-panel passes it in |

The reverse imports (preview → tasks) are removed by purification rather than
by moving files: `PreviewHeader` today imports `useTaskView`,
`useTaskDetailQuery`, `useDeployTask`, `WorkspaceHeader` from tasks — after
the refactor, `panels/preview-panel.tsx` (inside tasks) passes view state,
task data, and header slots into `Preview` as props, so `features/preview`
exports a self-contained iframe/inspector/logs kit with zero knowledge of
tasks. Same for `features/files`: `useSelectedFile` state arrives via props
from `files-panel.tsx` (its `useTaskSandboxStatusQuery` usage can stay if the
query hook moves to a shared api layer, or also become a prop).

**Considered and rejected**: a new top-level `pages/` layer
(feature-sliced-design style) and TanStack route co-location
(`routes/.../-components/`). Both satisfy the same import rule, but the team
prefers keeping the repo's `features/` convention; tasks-as-composition-feature
achieves the layering without introducing a new structural concept. The
trade-off accepted: when a second cross-feature page appears (dashboard,
compose — both already live under tasks), it also lands in `features/tasks`,
which will grow — revisit extracting a layer then.

### 1. One breakpoint check, at the top

```tsx
// task-page.tsx — the ONLY place useIsMobile() is called on this page
export function TaskPage({ kiosk = false }: TaskPageProps) {
  const isMobile = useIsMobile()
  if (kiosk) return <KioskTaskLayout />
  return isMobile ? <MobileTaskLayout /> : <DesktopTaskLayout />
}
```

Each view is a plain composition — no `isMobile` prop, no `mobileView` prop,
no `kiosk` prop below this line. `TaskPageLayout` is deleted.

```tsx
// desktop-task-layout.tsx (sketch)
export function DesktopTaskLayout() {
  return (
    <DesktopPanelControlsProvider>          {/* implements toggleChat via panel ref */}
      <ResizablePanelGroup direction="horizontal">
        <ResizablePanel defaultSize={40} minSize={10} maxSize={60} collapsible>
          <TaskLeftHeader trailing={null} />
          <ChatPanel />
        </ResizablePanel>
        <ResizableHandle withHandle />
        <ResizablePanel>
          <WorkspacePanel
            header={<WorkspaceHeader tabs={<ServerTabs />} viewportToggle={<ViewportToggle />} />}
            console="collapsible"           {/* view decides the console split exists */}
          />
        </ResizablePanel>
      </ResizablePanelGroup>
    </DesktopPanelControlsProvider>
  )
}

// mobile-task-layout.tsx (sketch)
export function MobileTaskLayout() {
  const { view } = useTaskView({ fallback: 'chat' })
  return (
    <MobilePanelControlsProvider>          {/* implements "show chat" as setView('chat') */}
      <div className="h-full flex flex-col">
        <TaskLeftHeader trailing={<ViewSwitcher />} />   {/* renamed MobileViewToggle */}
        {view === 'chat'
          ? <ChatPanel />
          : <WorkspacePanel header={<WorkspaceHeader />} console="hidden" />}
      </div>
    </MobilePanelControlsProvider>
  )
}
```

The kiosk variant stops being flags drilled through `TaskPage`
(`hideHeader`/`hideConsole`/`collapseToolCalls` decided far from where they
apply) and becomes composition-site configuration inside `kiosk-task-layout.tsx`
— the props can stay on `ChatContainer`/`Preview`, but only the kiosk view
sets them.

### 2. Replace ref wiring with semantic panel controls

The three ref escape hatches exist because panels need to trigger actions in
sibling panels, and the current shell can't express that. Define the actions
*semantically* and let each view provide the layout-appropriate implementation:

```tsx
// task-panel-controls.tsx
interface TaskPanelControls {
  toggleChat(): void        // desktop: collapse/expand panel; mobile: setView('chat')
  showWorkspace(view?: TaskView): void
  navigatePreview(path: string): void   // backed by usePreviewPath (URL) — no ref needed
  executeCommand(name: string): void    // registered by ChatPanel on mount
}
```

- `toggleChat` — desktop provider wraps the `ImperativePanelHandle`; mobile
  provider maps it to a view switch. `TaskKioskHeader` and Preview's
  `TOGGLE_CHAT` postMessage both call `usePanelControls().toggleChat()` and no
  longer care which layout they're in. `ChatToggleContext` is deleted.
- `navigatePreview` — `previewPath` is already URL state (`usePreviewPath`);
  the chat's "open URL in preview" click can write the URL directly (plus
  `setView(serverName)`), eliminating `previewNavigateRef` entirely.
- `executeCommand` — the one genuinely imperative action. A small
  register/invoke pair on the provider (ChatPanel registers its executor on
  mount) replaces `commandExecutorRef` + the `onCommandExecutorReady` prop.

### 3. Move event accumulation into a provider

`toSandboxEvent`, the dedup ring, `startupEvents`, and the historical-merge
effect (currently `task-page.tsx:104–177`) move into `SandboxEventsProvider`
under `TaskProviders`, exposed as `useSandboxEvents()` / `useStartupEvents()`.
`Preview`, `SandboxWorkspacePanel`/`LogsPanel` consume the hooks directly —
no more prop drilling, and events survive layout swaps because the provider
sits above the shells.

### 4. Purify the leaf components

- `WorkspaceHeader`: delete its `useIsMobile()`; it becomes a dumb frame with
  `tabs` / `center` / `right` slots. Desktop view passes `<ServerTabs />`;
  mobile view passes nothing.
- `PreviewHeader`: delete `useIsMobile()`; `ViewportToggle` becomes a slot the
  desktop view fills.
- `TaskLeftHeader`: delete the `isMobile &&` branch; gets a `trailing` slot,
  mobile view passes `<ViewSwitcher />` (renamed from `MobileViewToggle` —
  it's a view switcher, not inherently mobile).
- `useTaskView`: remove `isMobile` from the return; replace the
  mobile-defaults-to-chat branch with a `fallback` option each view supplies
  (mobile passes `'chat'`, desktop derives the default server as today).
- `Preview`: extract the console `ResizablePanelGroup` so Preview renders only
  iframe + overlays; the console split becomes a small `StackedPanel`-style
  composition owned by the views (desktop: collapsible split; mobile/kiosk:
  none). Same for `SandboxWorkspacePanel`'s internal split. This also removes
  the duplicated collapsed-vs-expanded console branches.

### 5. State that must survive a breakpoint swap

Swapping `MobileTaskLayout` ↔ `DesktopTaskLayout` on resize unmounts one tree.
Audit of what must survive and where it will live:

| State | Home after refactor | Survives swap? |
|---|---|---|
| view / preview path / file / viewport | URL search params (already) | ✅ |
| task, messages, sandbox status | TanStack Query cache (already) | ✅ |
| chat draft | localStorage (`useTaskDraftMessage`, already) | ✅ |
| `contextItems` | `TaskComposerProvider` (moved up from `TaskPage`) | ✅ |
| sandbox/startup events | `SandboxEventsProvider` (moved up) | ✅ |
| chat collapsed (desktop-only) | stays in `DesktopTaskLayout` — meaningless on mobile | n/a |

The preview iframe reloads when crossing the breakpoint — same behavior as
today (mobile already unmounts the workspace when switching to chat), so no
regression.

## Implementation plan (detailed, each phase ships independently)

Every phase ends green on `bun run type:check && bun run biome:check` and
`bun run test:unit`, plus a manual pass on the dev server: task page at
desktop width, at <768px (mobile view toggle, chat default), and the
`/tasks/$task-id/kiosk` route. Phases 1–2 must produce **zero visual change**.

### Phase 1 — Extract state out of `TaskPage` (no visual change)

**1a. `contexts/sandbox-events-provider.tsx` (new).** Move from
`task-page.tsx:28–177`: `toSandboxEvent`, `isStartupSubType`,
`isStartupEndSubType`, the `seenEventIds` dedup ref, `sandboxEvents` +
`startupEvents` state, the historical-merge effect
(`useHistoricalSandboxEvents`), and the `useTaskEventSubscription` callback.
Expose `useSandboxEvents(): SandboxEvent[]` and
`useStartupEvents(): StartupEvent[]`. Keep the single SSE subscription (one
provider = one subscription, as today). Mount it inside
`contexts/task-providers.tsx` under `TaskDataProvider`.

**1b. `contexts/task-composer-provider.tsx` (new).** Move `contextItems`
state, `handleToggleElementContext` (incl. the toasts), and the
previewPath→contextItems sync effect (`task-page.tsx:95, 229–303`). Expose
`useComposer(): { contextItems, setContextItems, toggleElementContext }`.
Mount in `task-providers.tsx`.

**1c. `contexts/task-panel-controls.tsx` (new).** Context with a stable value:

```tsx
interface TaskPanelControls {
  toggleChat(): void                     // impl injected by the active layout
  showWorkspace(view?: TaskView): void   // setView() under the hood
  navigatePreview(path: string): void    // setPreviewPath(); ensure a server view is active
  executeCommand(name: string): void     // invokes the registered executor (no-op if none)
  registerCommandExecutor(fn: (name: string) => void): () => void  // returns unregister
  registerChatToggle(fn: () => void): () => void
}
```

Registration uses refs internally (mirrors today's semantics exactly — the
executor "arrives" when ChatContainer mounts). `ChatContainer` calls
`registerCommandExecutor` in an effect and drops the
`onCommandExecutorReady` prop; the chat's `onPreviewUrlClick` prop is replaced
by calling `navigatePreview` directly (ChatContainer is inside tasks, so this
import is legal). In this phase `TaskPageLayout` registers its toggle via
`registerChatToggle` (replacing `onChatToggleReady`).

**Boundary rule for `features/preview`:** `Preview` KEEPS its callback props
(`onChatToggleRequest`, `onTroubleshootRequest`, `onToggleElementContext`,
`onNavigationReady`) so preview never imports tasks. The *values* now come
from one place: `TaskPage` (later `panels/preview-panel.tsx`) supplies
`() => controls.toggleChat()`, `() => controls.executeCommand('troubleshoot-preview')`,
`composer.toggleElementContext`, dropping the three refs from `TaskPage`.

**Result:** `task-page.tsx` loses ~180 lines of state/wiring; renders the
same tree.

### Phase 2 — Purify leaves, create `panels/`, break reverse imports (no visual change)

**2a. `lib/layout/` primitives (new).**
- `collapsible-console.tsx`: the vertical `ResizablePanelGroup` + collapsed
  40px bar currently duplicated inside `preview.tsx:640–794`. API:
  `{ main: ReactNode, console: ReactNode, mode: 'collapsible' | 'hidden' }`.
- `stacked-split.tsx`: the vertical split from `sandbox-workspace-panel.tsx`.

**2b. Purify headers (slot-driven, delete every `useIsMobile`).**
- `workspace-header.tsx`: drop `useIsMobile`, `activeView`, `onViewChange`,
  `servers`, `showToggle`; new props `{ tabs?, centerContent?, rightContent? }`.
  Callers render `<ServerTabs/>` into `tabs` (or nothing, on mobile).
- `preview-header.tsx`: drop `useIsMobile` + `useTaskView` +
  `useTaskDetailQuery` + `useDeployTask`; receive `viewportToggle?: ReactNode`,
  task data and publish action as props. Supplier is `preview-panel.tsx`.
- `task-left-header.tsx`: drop the `isMobile &&` branch; add
  `trailing?: ReactNode` slot (mobile view later passes the view toggle).

**2c. `panels/` (new).** Move the `workspaceContent` IIFE from
`task-page.tsx:322–399` into `panels/workspace-panel.tsx`
(loading/error/files/sandbox/server switch — content only, no splits). Add
`chat-panel.tsx` (ChatContainer + boundary, wired to composer/controls),
`files-panel.tsx` (passes `isSandboxReady`, retry mutation, selected-file
state into the explorer as props), `sandbox-panel.tsx`, `preview-panel.tsx`
(joins sandbox status + events + controls and passes them into `Preview`;
console arrangement via `collapsible-console`).

**2d. Kill the reverse imports.**
- `SANDBOX_VIEW` → `types/view-registry.ts` (re-export from preview
  temporarily if needed for one commit).
- `useHistoricalSandboxEvents`, `SandboxEvent`/`StartupEvent` types,
  `toSandboxEvent` → `features/events/`.
- `features/files`: replace `useSelectedFile` / `useTaskSandboxStatusQuery`
  imports with props from `files-panel.tsx`.
- `features/preview`: after 2b–2c, grep must show zero `@/features/tasks`
  imports from preview (`ServerLogsViewer` moves or arrives via props).

### Phase 3 — Split the shells (visible change: none intended, structure only)

- `layouts/task-page.tsx`: taskQuery error/loading handling +
  `const isMobile = useIsMobile()` → `<MobileTaskLayout/> | <DesktopTaskLayout/>`
  (kiosk route renders `<KioskTaskLayout/>` directly).
- `layouts/desktop-task-layout.tsx`: horizontal `ResizablePanelGroup`
  (chat 40/10–60 collapsible + workspace), expand-chat button, registers its
  panel-collapse fn via `registerChatToggle`. Header: `TaskLeftHeader` with
  `ServerTabs`/`ViewportToggle` slots filled.
- `layouts/mobile-task-layout.tsx`: single panel, `useTaskView({ fallback: 'chat' })`,
  `TaskLeftHeader trailing={<ViewSwitcher/>}`; `toggleChat` implemented as
  `setView('chat')`.
- `layouts/kiosk-task-layout.tsx`: workspace-left split, `TaskKioskHeader`
  (now calling `usePanelControls().toggleChat` — `ChatToggleContext` deleted),
  sets `hideHeader/hideConsole/collapseToolCalls` at the composition site.
- Delete `components/task-page.tsx` + `components/task-page-layout.tsx`.
- `use-task-view.ts`: remove `useIsMobile` + the mobile-default branch;
  signature becomes `useTaskView({ servers?, fallback? })`.

### Phase 4 — Moves, rename, enforcement

- `lib/components/chat/` → `features/tasks/components/chat/` (~14 import
  sites, mechanical; Biome fixes import order).
- Apply the remaining terminology renames (see Terminology section). Some
  land naturally in earlier phases — `ServerTabs` → `ViewTabs` and
  `TaskLeftHeader` → `TaskHeader` during 2b, `SandboxEvent` →
  `LifecycleEvent` during 2d — the rest here: `mobile-view-toggle.tsx` →
  `view-switcher.tsx`, `ChatView` → `ChatScrollArea`, `ViewportMode`/
  `ViewportToggle`/`usePreviewViewport` → `DeviceMode`/`DeviceToggle`/
  `usePreviewDevice` (URL param `viewport` → `device`), `TaskDetail` →
  `TaskPageRoute`.
- Wire `TaskUIProvider`'s stored panel sizes into `DesktopTaskLayout`'s
  `ResizablePanelGroup` (they exist but are unused today) — or delete them.
- Enforcement in `apps/frontend/biome.json` overrides (shape sketch; verify
  against the installed Biome version):

```jsonc
{
  "overrides": [
    {
      "includes": ["src/features/!(tasks)/**"],
      "linter": { "rules": { "style": { "noRestrictedImports": {
        "level": "error",
        "options": { "paths": {
          "@/features/tasks": "Only features/tasks composes other features.",
          "@/features/tasks/**": "Only features/tasks composes other features."
        } }
      } } } }
    }
  ]
}
```

- Update `CLAUDE.md` / `.claude/rules/frontend-conventions.md` with the
  layer rules (layouts/panels/contexts, tasks-as-composition-feature).

### Risks / things to verify while implementing

- **`navigatePreview` semantics**: today Preview hands out an imperative
  navigate fn (`onNavigationReady`). Confirm Preview reacts to external
  `previewPath` (URL) changes; if it only pushes path outward, add a
  controlled effect keyed on `previewPath` before deleting the ref plumbing.
- **Kiosk panel order** (workspace left) and the two expand-button variants
  are behavior to preserve exactly — cover with a snapshot/interaction test
  before deleting `TaskPageLayout`.
- **Existing tests** reference moved components
  (`components/__tests__/task-*.test.tsx`, `lib/hooks/__tests__/use-auto-scroll.integration.test.ts`
  imports `lib/components/chat`) — move/update alongside.
- **`useSandboxStatus` dedup**: panels each calling it is fine (shared query
  cache), but keep the `enabled: status !== 'closed'` gate when it moves.
- **Mobile default view flash**: `useTaskView`'s fallback previously depended
  on `isMobile` inside the hook; after the split, the *view component* chooses
  the fallback, so a resize across 768px changes the effective default —
  acceptable, but don't write the default into the URL on mount.

## Trade-offs considered

- **Separate shells vs. CSS-only**: CSS breakpoints can't express
  "resizable split with imperative collapse" vs. "one panel at a time with a
  toggle" without mounting both trees (double iframes, double SSE-driven
  lists). JS branching at a single point is the standard pattern for
  divergent interaction models; the cost (unmount on resize across 768px) is
  rare in practice and fully mitigated by the state audit above.
- **Some duplication between views is the point.** The three view files will
  share panel children but repeat small amounts of arrangement JSX. That
  duplication is cheaper than the current conditional coupling, and lets
  mobile evolve (e.g. to a swipeable pager or bottom sheet) without touching
  desktop.
- **Context vs. zustand for panel controls**: a task-scoped context is enough
  here (few consumers, actions not state, no re-render pressure since the
  value is stable callbacks). Revisit only if cross-panel *reactive* state
  grows.

## Appendix: full inventory of `features/tasks` (what to refactor vs. leave)

~9,000 lines total (excl. tests). The feature actually contains **three
sub-domains**; this refactor targets only the first:

1. **Task page (detail view)** — the refactor target, ~2,400 lines
2. **Task creation / compose** — untouched
3. **Dashboard / task list** — untouched

### 1. Task page cluster (refactor target)

| File | Lines | Disposition |
|---|---|---|
| `components/task-page.tsx` | 426 | **Dissolve**: state → contexts (phase 1), workspace switch → `panels/workspace-panel` (2c), shell choice → `layouts/task-page` (3) |
| `components/task-page-layout.tsx` | 179 | **Delete** (phase 3) — replaced by the three `layouts/` |
| `components/__stories__/task-page-layout.stories.tsx` | 47 | Replace with one story per layout |
| `components/chat-view/chat-container.tsx` | 749 | **Keep, slim**: drop `onCommandExecutorReady`/`onPreviewUrlClick` props (use panel-controls), read composer context. Biggest file in the feature — later candidate to split (dialogs vs. input vs. history wiring) |
| `components/chat-view/command-buttons.tsx` | 63 | Keep — already pure |
| `components/workspace-header.tsx` | 56 | **Purify** (2b): drop `useIsMobile` + ServerTabs knowledge → `tabs` slot |
| `components/mobile-view-toggle.tsx` | 84 | **Rename** → `view-switcher.tsx` (4); placed only by `MobileTaskLayout` |
| `components/sandbox-workspace-panel.tsx` | 66 | **Becomes** `panels/sandbox-panel.tsx`; internal split → `lib/layout` |
| `components/task-control-panel.tsx` | 173 | Keep — already fully self-contained (own hooks, `className`-only props) |
| `components/server-logs-viewer/` | 90 | **Move or invert**: imported by preview's `LogsPanel` (reverse import). Move to `features/events/` or inject via props |
| `components/task-header/task-left-header.tsx` | 115 | **Purify** (2b): drop `isMobile` branch → `trailing` slot |
| `components/task-header/task-kiosk-header.tsx` | 66 | **Rewire** (3): `useChatToggle` → `usePanelControls().toggleChat` |
| `components/task-header/task-info-section.tsx` | 273 | Keep |
| `components/task-header/edit-task-dialog.tsx` | 155 | Keep |
| `components/task-header/create-repo-{button,dialog}.tsx` | 179 | Keep |
| `components/task-header/scheduled-jobs-pill.tsx` | 44 | Keep |
| `components/minified-event-viewer/` (4 files) | 412 | Event-rendering; candidate to move to `features/events/` with the event types (2d), not urgent |
| `components/task-skeletons.tsx` | 48 | Keep |
| `contexts/task-data-provider.tsx` | 138 | Keep |
| `contexts/task-ui-provider.tsx` | 90 | **Wire or delete** (4): panel sizes stored but unused |
| `contexts/task-providers.tsx` | 41 | Grows: mounts the three new providers (1a–1c) |
| `hooks/use-task-view.ts` | 195 | **Strip `isMobile`** (3); add `fallback` option |
| `hooks/use-sandbox-status.ts` | 103 | Keep |
| `hooks/use-task-queries.ts` | 73 | Keep |
| `hooks/use-task-inputs.ts` | 304 | Keep (composer domain — pairs with new `task-composer-provider`) |
| `hooks/use-task-draft-message.ts` | 70 | Keep |
| `hooks/use-task-context.ts` | 51 | Keep |
| `types/task-search-schema.ts` | 21 | Keep (source of truth for URL state) |

New files (from the plan): `layouts/` (4), `panels/` (5), `contexts/` (+3),
`types/view-registry.ts`, `lib/layout/` (2).

### 2. Creation / compose cluster (out of scope)

`components/compose/` (389 lines, 4 files), `components/create-task-form.tsx`
(270), `components/create-task-dialog.tsx` (47),
`components/create-task/create-code-task.tsx` (518),
`contexts/compose-task-provider.tsx` (103), `hooks/use-compose-task-form.ts`
(343). — Its own mini-composition (imports figma, repositories, uploads);
legal under the tasks-composes rule. If it grows, give it the same
layouts/panels treatment later.

### 3. Dashboard / list cluster (out of scope)

`components/dashboard/` (task-dashboard 304, task-filters 175, task-table
316), `components/task-card/task-card.tsx` (429),
`components/task-dialog/` (40), `components/task-connections-section.tsx`
(61), `components/task-status-icon.tsx` (71), `utils/` (538 across 5 files),
`api/tasks.ts` (225). — The task-list "page" living in the same feature;
unaffected by this refactor.

### Cross-cutting hook worth a follow-up

`hooks/use-tasks.ts` (716 lines) mixes list queries, task mutations, message
sending/stopping, deployment, and commands in one file. Not required for the
layout refactor, but splitting it (`use-task-mutations`, `use-task-messages`,
`use-task-commands`) would pay down the same mixed-concern debt at the hook
layer.
