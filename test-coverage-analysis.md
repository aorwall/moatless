# Frontend Test Coverage Analysis

## Summary
Total source files (excluding tests, index, types): 112
Files with tests: 33
Files without tests: 79
Coverage percentage: ~29%

## Files Without Tests - By Priority

### 🔴 High Priority - Core Features & Components

#### 1. Navigation Components
- `features/navigation/components/app-sidebar.tsx` - Main navigation component
- `features/navigation/components/breadcrumbs.tsx` - Breadcrumb navigation
- `features/navigation/components/sidebar-tasks.tsx` - Task list in sidebar
- `features/navigation/components/sidebar-repositories.tsx` - Repository list in sidebar
- `features/navigation/components/sidebar-commands.tsx` - Commands list in sidebar

#### 2. Repository Management
- `features/repositories/hooks/use-repositories.ts` - Core repository data hook
- `features/repositories/components/repository-list.tsx` - Repository listing component
- `features/repositories/components/repository-form.tsx` - Repository create/edit form
- `features/repositories/api/repositories.ts` - Repository API calls
- `features/repositories/hooks/use-detect-framework.ts` - Framework detection hook

#### 3. Task Core Components
- `features/tasks/components/task-page.tsx` - Main task page component
- `features/tasks/components/create-task-form.tsx` - Task creation form
- `features/tasks/components/create-task-dialog.tsx` - Task creation dialog
- `features/tasks/contexts/task-data-provider.tsx` - Task data context
- `features/tasks/contexts/task-events-provider.tsx` - Task events context
- `features/tasks/api/tasks.ts` - Task API calls

#### 4. File Viewer
- `features/file-viewer/components/file-viewer.tsx` - Main file viewer component
- `features/file-viewer/components/file-tree.tsx` - File tree component
- `features/file-viewer/hooks/use-file-tree-state.ts` - File tree state management
- `features/file-viewer/api/use-file-search.ts` - File search hook
- `features/file-viewer/api/use-file-content.ts` - File content fetching

#### 5. Event System
- `features/events/contexts/global-event-stream-provider.tsx` - Global SSE event provider
- `features/events/utils/event-handlers.ts` - Event handling utilities

### 🟡 Medium Priority - Important but Less Critical

#### 1. Preview Feature
- `features/preview/components/preview.tsx` - Main preview component
- `features/preview/hooks/use-preview.tsx` - Preview hook
- `features/preview/hooks/use-server.ts` - Server management hook
- `features/preview/components/preview-toolbar.tsx` - Preview toolbar (has partial tests)
- `features/preview/components/inspector-panel.tsx` - Inspector panel
- `features/preview/components/logs-panel.tsx` - Logs panel

#### 2. Git Diff Viewer
- `features/tasks/components/git-diff-viewer/git-diff-viewer.tsx` - Git diff viewer (has partial tests)
- `features/tasks/components/git-diff-viewer/git-diff-file-tree.tsx` - Git diff file tree

#### 3. Commands Feature
- `features/commands/hooks/use-commands.ts` - Commands data hook
- `features/commands/components/command-list.tsx` - Command listing
- `features/commands/components/command-form.tsx` - Command form
- `features/commands/api/commands.ts` - Commands API

#### 4. Claude Code Integration
- `features/claude-code/hooks/use-claude-code.ts` - Claude Code integration hook
- `features/claude-code/api/claude-code.ts` - Claude Code API

#### 5. Server Logs
- `features/tasks/components/server-logs-viewer/server-logs-viewer.tsx` - Server logs viewer

### 🟢 Lower Priority - UI Components & Utilities

#### 1. Event Viewer Tool Calls (many individual components)
- Various tool call components in `event-viewer/event-components/tool-calls/`
- These are mostly display components with minimal logic

#### 2. Skeleton Components
- `features/file-viewer/components/file-tree-skeleton.tsx`
- `features/file-viewer/components/file-viewer-skeleton.tsx`
- `features/tasks/components/task-skeletons.tsx`
- `features/tasks/components/chat-view/chat-view-skeleton.tsx`

#### 3. Utility Functions
- `features/tasks/utils/format-duration.ts`
- `features/tasks/utils/git-sync-status.tsx`
- `features/tasks/utils/query-helpers.ts`
- `features/tasks/utils/task-status.tsx`
- `features/file-viewer/utils/file-tree-utils.ts`

#### 4. Settings Components
- `features/settings/components/repository-list.tsx`
- `features/settings/hooks/use-repositories-settings.ts`

## Files With Tests

### Well-Tested Areas
1. **Tasks Feature** - Good coverage for core task components
   - task-list, task-header, task-card, event-viewer
   - chat-view, chat-input components
   - Hooks: use-tasks, use-task-data

2. **Preview Feature** - Partial coverage
   - preview-store, script-health-indicator
   - Some toolbar tests

3. **Event Components** - Some coverage
   - message-event, tool-call-event

## Recommendations

### Immediate Testing Priorities (Top 10)
1. **app-sidebar.tsx** - Core navigation component used on every page
2. **global-event-stream-provider.tsx** - Critical for real-time updates
3. **use-repositories.ts** - Core data hook for repository management
4. **task-page.tsx** - Main task detail page component
5. **file-viewer.tsx** - Important for code viewing functionality
6. **create-task-form.tsx** - Task creation is a core feature
7. **repository-form.tsx** - Repository management is essential
8. **task-data-provider.tsx** - Critical context for task data
9. **use-file-tree-state.ts** - Complex state management logic
10. **use-preview.tsx** - Preview functionality hook

### Testing Strategy
1. Focus on integration tests for route components and pages
2. Unit test hooks with complex logic
3. Integration test context providers
4. Mock external dependencies (APIs, SSE connections)
5. Test error states and edge cases
6. Use the project's testing patterns (see CLAUDE.md)