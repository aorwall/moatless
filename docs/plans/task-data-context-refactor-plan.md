# Task Data Context Refactor Plan

## Goals
- Decouple network fetching from `TaskDataProvider`, keeping the provider focused on SSE buffering and task-level coordination.
- Align the task SSE pipeline with `apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx`, avoiding duplicate listeners and clarifying ownership.
- Ensure components access server state through dedicated React Query hooks, enabling granular caching and easier composition.
- Document and preserve UI-state isolation (`TaskUIProvider`) while reducing redundant re-renders and duplicated query logic.
- Provide a clear migration path for existing consumers (`TaskPage`, `ChatContainer`, headers, file viewers) so they rely on the new hook layer.
- Capture rationale for workspace polling and derived flags inside the code and supporting docs so their use is intentional and testable.

## Current Pain Points
- `TaskDataProvider` performs multiple `useQuery` calls and exposes wide context values, causing every consumer to re-render for any data change.
- Slice hooks (`useTaskStatus`, `useTaskIterations`, etc.) reissue queries or duplicate selectors, leading to inconsistent configuration.
- Temporary-ID guards and optimistically-created tasks complicate fetch gating and error handling.
- Streaming responsibilities are split between the task provider and `GlobalEventStreamProvider`, which increases duplication and race conditions when both subscribe to the same task events.
- Tests mock provider internals instead of targeting the shared hooks, making refactors risky.

## Context & Data Flow Snapshot
- **GlobalEventStreamProvider**: owns the single SSE connection, fan-outs task, workspace, and system events.
- **TaskDataProvider (target state)**: accepts a `taskId`, subscribes to the global stream, buffers task-specific events, and exposes `{ taskId, streamingEvents, isStreaming, streamingError }` only.
- **React Query hooks**: `useTaskDetailQuery`, `useTaskGitDiffQuery`, `useTaskWorkspaceStatusQuery`, and follow-up selectors (`useTaskMetadata`, etc.) source network data.
- **Task UI Context**: manages purely local UI state (panels, filters, composer drafts) and should not depend on network fetches.
- **Consumers**: assemble the pieces—task ID from provider, queries from hooks, SSE stream for live updates, and UI context for per-view state.

## Guiding Principles
1. **Single responsibility**: the provider should only surface `taskId` and streaming event buffers.
2. **Hook-first data access**: React Query hooks own fetch logic (stale time, retries, polling); consumers opt in explicitly.
3. **Memoized selectors**: derive lightweight helpers (e.g., `useTaskMetadata`) by composing hooks, not context state.
4. **Incremental rollout**: migrate consumers feature-by-feature to avoid large bang refactors.

## Work Breakdown

### Phase 0 – Prep
- [x] Confirm query hooks (`useTaskDetailQuery`, `useTaskGitDiffQuery`, `useTaskWorkspaceStatusQuery`) cover all current options.
- [x] Document existing consumers and their data needs (task status, metadata, workspace, git diff, iterations).
- [x] Map streaming event consumers and how they currently access the global SSE provider versus task provider.

#### Consumer Data Needs Inventory
| Consumer (file:line) | Fields from `useTaskData` | Primary use | Replacement plan |
| --- | --- | --- | --- |
| apps/frontend/src/features/tasks/components/task-page.tsx:19 | `isLoading`, `error` | Controls shell loading/error fallbacks for the task route | Derive from `useTaskDetailQuery(taskId)` or a thin `useTaskLoadState(taskId)` wrapper so the provider no longer hydrates query state. |
| apps/frontend/src/features/tasks/contexts/task-providers.tsx:13 | `vibe` | Passes task vibe into `TaskUIProvider` to seed UI defaults | Read via `useTaskDetailQuery(taskId)` and pass the value down explicitly; provider should only supply `taskId`. |
| apps/frontend/src/features/tasks/components/task-header/task-left-header.tsx:7 | `status`, `agentType`, `repositoryId`, `iterations`, `vibe`, `templateId`, `name` | Renders task title/status metadata in the header | Use `useTaskId()` + `useTaskDetailQuery(taskId)` (or `useTaskMetadata`) to read only the required fields. |
| apps/frontend/src/features/tasks/components/chat-view/chat-container.tsx:40 | `status`, `iterations`, `workspaceStatus` | Drives message placeholders, status banners, send-button gating | Combine `useTaskDetailQuery(taskId)` for `status/iterations` with `useWorkspaceStatus(taskId, task)` (polling only while open). |
| apps/frontend/src/features/file-viewer/components/file-explorer-header.tsx:18 | `taskId`, `status` | Controls merge/sync/close affordances and passes task ID into mutations | Use `useTaskId()` for identity, `useTaskStatus()` for task state, and `useTaskWorkspaceStatusQuery(taskId, { enabled: status === 'open' })` for git status polling. |
| apps/frontend/src/features/preview/components/preview-header.tsx:62 | `vibe`, `iterations` | Detect vibe tasks and surface latest iteration preview URLs | Read from `useTaskDetailQuery(taskId)` (or a memoized selector) so Preview header does not depend on provider query state. |

#### Streaming Event Consumers (current state)
- `apps/frontend/src/features/tasks/contexts/task-data-provider.tsx:57` subscribes to task-scoped events via `useTaskEventSubscription`, buffers the last 20 events, and exposes stream status from `useGlobalEventStream`.
- `apps/frontend/src/features/tasks/components/minified-event-viewer/task-minified-last-events-display.tsx:9` consumes `useTaskEvents`, `useIsAgentRunning`, and `useLatestContextTokens` to show the rolling event list in the task UI.
- `apps/frontend/src/features/tasks/components/task-card/task-card.tsx:126` uses `useTaskEventSubscription` directly to enrich task cards with live agent/tool messages on the dashboard.
- `apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx:90-210` owns the single SSE connection, updates React Query caches, and exposes subscription helpers (`subscribeToTask`, `subscribeToType`) that both task cards and the task provider consume.

### Phase 1 – Provider Slimming
- [x] Strip `TaskDataProvider` down to `{ taskId, streamingEvents, isStreaming, streamingError }`.
- [x] Update context types and helper `useTaskDataContext` accordingly.
- [x] Ensure `TaskDataProvider` delegates SSE subscription to `GlobalEventStreamProvider` (or vice versa) to avoid double listeners.
- [x] Remove redundant slice hooks or rewrite them to call the new query hooks directly.
- [x] Delete remaining temporary-ID guards in task contexts (confirm no regressions after recent removals).

### Phase 2 – Consumer Migration
- [x] `TaskPage`: replace `useTaskData()` with explicit calls to task + workspace hooks.
- [x] `ChatContainer`: fetch status/iterations/workspace via hooks; keep streaming via `useTaskEvents`.
- [x] `TaskLeftHeader` & `WorkspaceHeader`: read metadata/status from hooks.
- [x] File viewer + preview components: swap to hook-based git diff/workspace state.
- [x] Tests: update mocks to target `useTaskDetailQuery` etc. instead of provider output.
- [x] Document workspace polling triggers directly in components that depend on them, linking to code comments.

### Phase 3 – Cleanup
- [x] Remove obsolete helper hooks that no longer add value (`useTaskLoadingError`, etc.) or re-implement them as thin wrappers over query hooks.
- [x] Delete temp-ID fetch guards and ensure optimistic flows rely on real responses (already done for vibe create; verify other mutations).
- [x] Audit logging (remove provider console logs) and ensure SSE buffer size is configurable/tested.
- [x] Update `docs/patterns/react-query.md` with the new context/hook division and the removal of temporary IDs.

### Phase 4 – Validation
- [x] Run unit/integration tests for tasks, file viewer, preview, and chat features. *(vitest: task-page-toggle unit suite)*
- [ ] Measure render counts on `TaskPage` (React Profiler) to verify fewer unnecessary updates.
- [x] Document the new architecture in `docs/patterns/react-query.md` and feature README.
- [x] Capture rationale for workspace polling and derived flags inside code comments and link from docs.

## Workspace Polling & Derived Flags – Current Usage
- **Polling contract**: `apps/frontend/src/features/tasks/hooks/use-task-queries.ts:61-96` keeps `useTaskWorkspaceStatusQuery` refetching every 3 s only while `environmentStatus` is `initializing`/`cleaning` or `agentState` is `running`, preventing unnecessary polling once the workspace settles.
- **Task shell defaults**: `apps/frontend/src/features/tasks/components/task-page.tsx:22-41` reads workspace capabilities to auto-switch the right panel to Preview when server management is available, and defers rendering until `useTaskDetailQuery` resolves.
- **Chat UX gating**: `apps/frontend/src/features/tasks/components/chat-view/chat-container.tsx:140-210` uses workspace status to gate send actions, surface “workspace initializing/error” banners, and adjust placeholders; derived `isAgentRunning` (via `useIsAgentRunning`) prevents duplicate start/stop requests.
- **File operations**: `apps/frontend/src/features/file-viewer/components/file-explorer-header.tsx:18-188` combines `status`, `isAgentRunning`, and polled `workspaceStatus.gitStatus` to enable/disable merge, sync, and close buttons with descriptive tooltips.
- **Preview controls**: `apps/frontend/src/features/preview/components/preview-header.tsx:62-140` uses iterations from the task query to locate the latest preview URL and determine when “Publish” actions are valid for vibe tasks.
- **Derived flags audit**: `isAgentRunning` is actively consumed (chat, file explorer, minified events), while `canExecute`/`isComplete` currently appear only in tests—flag them for removal or conversion to explicit selectors during refactor so consumers opt-in intentionally.

## Risks & Mitigations
- **Double fetching**: During migration, components might call hooks twice (legacy + new). Mitigation: migrate feature-by-feature and delete legacy calls immediately.
- **Test brittleness**: Mock-heavy tests may break when provider output changes. Mitigation: convert to integration-style tests or mock query hooks.
- **Missing SSE data**: Ensure provider still registers `useTaskEventSubscription` even after state pruning.

## Success Criteria
- Provider no longer imports `useTaskDetailQuery` et al.; only consumers do.
- No component re-renders due to unrelated query updates when using task contexts.
- Tests rely on query hooks and maintainers understand the split via updated documentation.
- Workspace polling and derived flags (`canExecute`, `isComplete`, `isAgentRunning`) are documented with concrete call sites and covered by tests.

## Status Tracking
- Drive execution via a dedicated branch (e.g., `feat/task-context-refactor`).
- Update this plan with checkmarks as phases/steps complete.
