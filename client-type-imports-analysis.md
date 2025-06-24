# Client Type Imports Migration Analysis

## Overview
This analysis identifies all files in `client/src/features/` that import from `@/types/` and need migration.

## Summary of Type Usage

### Most Commonly Used Type Files
1. **event.ts** - 21 files (most critical)
2. **environment.ts** - 12 files
3. **task.ts** - 11 files
4. **agent.ts** - 2 files
5. **claude-code.ts** - 2 files
6. **git-diff.ts** - 2 files

### Key Observations

#### Property Naming Patterns
The codebase uses **MIXED naming conventions**:
- **snake_case** in type definitions (e.g., `task_id`, `event_id`, `created_at`)
- **camelCase** in component usage (e.g., `task.id`, `task.createdAt`)
- This suggests a transformation layer exists between API responses and component props

#### Type Import Patterns
```typescript
// Common patterns found:
import type { Event } from '@/types/event'
import type { Task, TaskMetadata, WorkspaceStatus } from '@/types/task'
import type { Environment, EnvironmentResponse } from '@/types/environment'
import { WorkspaceStatus } from '@/types/task' // Enum import
import { AgentEventType } from '@/types/event' // Enum import
```

## Files That Need Migration

### Tasks Feature (26 files)
```
/features/tasks/api/
  - ndjson-sse.ts → Event
  - tasks.ts → AgentTriggerRequest/Response, FileSearchRequest/Response, Event, GitDiffResponse, ServerStatus, TaskMessages, TaskMetadata, WorkspaceStatusResponse

/features/tasks/components/
  - chat-input/environment-selector.tsx → EnvironmentResponse
  - chat-view/chat-view.tsx → Event
  - event-viewer/event-list.tsx → Event
  - event-viewer/event-components/event-header.tsx → Event
  - event-viewer/event-components/input-event.tsx → Event
  - event-viewer/event-components/message-event.tsx → Event
  - event-viewer/event-components/result-event.tsx → Event
  - event-viewer/event-components/tool-call-event.tsx → Event
  - event-viewer/event-components/workspace-event.tsx → Event
  - file-autocomplete.tsx → FileSearchResult
  - task-card.tsx → Task
  - task-header.tsx → Task
  - task-list.tsx → Task

/features/tasks/hooks/
  - use-claude-code-parser.ts → Event
  - use-task-events.ts → Event
  - use-task-streaming.ts → Event
  - use-tasks.ts → TaskMetadata

/features/tasks/stores/
  - task-event-store.ts → Event
  - task-ui-store.ts → WorkspaceStatus

/features/tasks/utils/
  - merge-tool-events.ts → Event, AgentEventType

/features/tasks/__tests__/
  - Various test files importing Event, Task types
```

### Environments Feature (6 files)
```
/features/environments/api/
  - environments.ts → EnvironmentResponse, CreateEnvironmentRequest, UpdateEnvironmentRequest

/features/environments/components/
  - code-execution-config.tsx → CodeExecution, SetupScriptCommand, SetupScriptResponse
  - environment-form.tsx → CreateEnvironmentRequest, EnvironmentResponse, UpdateEnvironmentRequest
  - environment-list.tsx → EnvironmentResponse

/features/environments/hooks/
  - use-environments.ts → EnvironmentResponse, CreateEnvironmentRequest, UpdateEnvironmentRequest
```

### Settings Feature (3 files)
```
/features/settings/components/
  - environment-form.tsx → Environment
  - environment-list.tsx → Environment

/features/settings/hooks/
  - use-environments-settings.ts → Environment
```

### Claude Code Feature (2 files)
```
/features/claude-code/api/
  - claude-code.ts → ClaudeCodeTriggerRequest, ClaudeCodeTriggerResponse

/features/claude-code/hooks/
  - use-claude-code.ts → ClaudeCodeTriggerRequest, ClaudeCodeTriggerResponse
```

## Migration Requirements

### Critical Types to Migrate First
1. **Event** - Most widely used (21 files)
2. **TaskMetadata, Task** - Core to task functionality
3. **EnvironmentResponse** - Used across multiple features
4. **WorkspaceStatus** - Enum used in multiple places

### Property Transformations Needed
The migration must handle the snake_case → camelCase transformation:
- `task_id` → `id` (in Task type)
- `created_at` → `createdAt`
- `last_message_at` → `lastMessageAt`
- `agent_type` → `agentType`
- `environment_name` → `environmentName`
- `num_turns` → `numTurns`
- `total_cost_usd` → `totalCostUsd`
- `workspace_status` → `workspaceStatus`

### Enum Migrations
Several enums need to be migrated:
- `AgentEventType` (MESSAGE, ERROR, RESULT, WORKSPACE)
- `WorkspaceStatus` (NOT_SETUP, SETTING_UP, IDLE, RUNNING)
- `MessageType` (USER, ASSISTANT, RESULT)
- `Role` (USER, ASSISTANT, SYSTEM)
- `ContentType` (TEXT, TOOL_RESULT, TOOL_USE)
- `ResultSubtype` (SUCCESS, ERROR)

### Import Path Updates
All imports need to change from:
```typescript
import type { X } from '@/types/file'
```
To:
```typescript
import type { X } from '@moatless/shared/types/file'
```

## Migration Strategy
1. Start with the shared package type definitions
2. Update import paths in batches by feature
3. Test each feature after migration
4. Handle any property naming transformations in API layer
5. Ensure type compatibility between client and server