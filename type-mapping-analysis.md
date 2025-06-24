# Type Mapping Analysis: Client → Shared Types Migration

This document provides a detailed analysis of the differences between client types and shared types, mapping conversion requirements for the migration.

## Overview

The client uses mixed naming conventions (snake_case and camelCase) and legacy structure, while shared types use consistent camelCase and modern TypeScript patterns. The migration needs to handle:

1. **Naming Convention Changes**: snake_case → camelCase
2. **Type Structure Differences**: Field additions/removals, type changes
3. **Date Handling**: String → Date object conversion
4. **Response Wrapper Changes**: Different API response formats

## Task Types Analysis

### TaskIteration

| **Client Type** | **Shared Type** | **Conversion Required** |
|-----------------|-----------------|-------------------------|
| `iteration_id: string` | `iterationNumber: number` | ✅ **BREAKING**: Field name and type change |
| `start_time: string` | `startTime: Date` | ✅ Snake→camel case + string→Date conversion |
| `end_time: string \| null` | `endTime?: Date` | ✅ Snake→camel case + string→Date conversion |
| `status: 'active' \| 'completed' \| 'error'` | `status: 'active' \| 'completed' \| 'error'` | ✅ **Compatible** |
| `message_count: number` | `messageCount: number` | ✅ Snake→camel case |
| `cost_usd: number` | `costUsd: number` | ✅ Snake→camel case |
| `initial_message: string` | `initialMessage?: string` | ✅ Snake→camel case + optional change |
| `error_message: string \| null` | `errorMessage?: string` | ✅ Snake→camel case + null→optional |
| `result: string \| null` | `result?: string` | ✅ Null→optional |
| `duration_ms: number` | `durationMs?: number` | ✅ Snake→camel case + optional change |
| `git_diff: string \| null` | `gitDiff?: string` | ✅ Snake→camel case + null→optional |
| **Missing** | `merged?: boolean` | ✅ **NEW FIELD** in shared |

### TaskMetadata vs TaskResponse

| **Client Type** | **Shared Type** | **Conversion Required** |
|-----------------|-----------------|-------------------------|
| `task_id: string` | `taskId: string` | ✅ Snake→camel case |
| `title: string` | `title?: string` | ✅ Required→optional change |
| `created_at: string` | `createdAt: Date` | ✅ Snake→camel case + string→Date |
| `last_message_at: string` | `lastMessageAt: Date` | ✅ Snake→camel case + string→Date |
| `agent_type: string` | `agentType: string` | ✅ Snake→camel case |
| `environment_id: string` | `environmentId: string` | ✅ Snake→camel case |
| `environment_name: string` | `environmentName?: string` | ✅ Snake→camel case + required→optional |
| `is_running: boolean` | `isRunning: boolean` | ✅ Snake→camel case |
| `status: 'active' \| 'completed' \| 'error'` | `status: 'active' \| 'completed' \| 'error' \| 'closed'` | ✅ **NEW STATUS**: 'closed' added |
| `num_turns: number` | `numTurns: number` | ✅ Snake→camel case |
| `total_cost_usd: number` | `totalCostUsd: number` | ✅ Snake→camel case |
| `iterations: Array<TaskIteration>` | `iterations?: TaskIterationResponse[]` | ✅ Required→optional + type change |
| **Missing** | `workspaceStatus?: string` | ✅ **NEW FIELD** in shared |

### Client Legacy Task Type

The client has a separate `Task` interface for UI display that needs mapping:

| **Client Task** | **Shared TaskResponse** | **Notes** |
|-----------------|-------------------------|-----------|
| `id: string` | `taskId: string` | Field name change |
| `title: string` | `title?: string` | Required→optional |
| `createdAt: string` | `createdAt: Date` | String→Date conversion |
| `lastMessageAt: string` | `lastMessageAt: Date` | String→Date conversion |
| `agentType: string` | `agentType: string` | ✅ Compatible |
| `environmentName: string` | `environmentName?: string` | Required→optional |
| `status: 'active' \| 'completed' \| 'error'` | `status: 'active' \| 'completed' \| 'error' \| 'closed'` | New 'closed' status |
| `numTurns: number` | `numTurns: number` | ✅ Compatible |
| `totalCostUsd: number` | `totalCostUsd: number` | ✅ Compatible |
| `workspaceStatus: string` | `workspaceStatus?: string` | Types compatible |
| `time: string` (legacy) | **Not in shared** | Legacy field to remove |
| `project: string` (legacy) | **Not in shared** | Legacy field to remove |
| `changes?: { added: number; removed: number }` | **Not in shared** | Legacy field to remove |

## Event Types Analysis

### Event Structure Comparison

| **Client Event** | **Shared Event** | **Conversion Required** |
|------------------|------------------|-------------------------|
| `event_id: string` | `eventId: string` | ✅ Snake→camel case |
| `type: AgentEventType \| string` | `type: EventType` | ✅ **BREAKING**: Enum change |
| `taskId: string` | `taskId: string` | ✅ **Compatible** |
| `iterationNumber: number` | `iterationNumber?: number` | ✅ Required→optional |
| `timestamp: string` | `timestamp: Date` | ✅ String→Date conversion |
| `isError: boolean` | `isError: boolean` | ✅ **Compatible** |
| `subType: string` | `subType?: string` | ✅ Required→optional |
| `contextTokens: number` | `contextTokens?: number` | ✅ Required→optional |
| `numTurns: number` | `numTurns?: number` | ✅ Required→optional |
| `totalCost: number` | `totalCost?: number` | ✅ Required→optional |
| `payload: Record<string, unknown>` | `payload: EventPayload` | ✅ **Compatible** |

### Event Type Enum Changes

**Client AgentEventType:**
```typescript
enum AgentEventType {
  MESSAGE = 'message',
  ERROR = 'error', 
  RESULT = 'result',
  WORKSPACE = 'workspace',
}
```

**Shared EventType:**
```typescript
enum EventType {
  INPUT = 'input',        // NEW
  MESSAGE = 'message',    // Same
  START = 'start',        // NEW
  STOP = 'stop',          // NEW
  RESULT = 'result',      // Same
  ERROR = 'error',        // Same
  WORKSPACE = 'workspace', // Same
  SERVER = 'server',      // NEW
  TOOL = 'tool',          // NEW
  COMPLETION = 'completion', // NEW
}
```

**Migration Impact**: 
- ✅ Existing client event types are preserved
- ✅ New event types added for enhanced functionality
- ⚠️ Need to handle unknown event types gracefully

## Environment Types Analysis

### Environment Structure

| **Client Environment** | **Shared Environment** | **Conversion Required** |
|------------------------|-------------------------|-------------------------|
| `id: string` | `id: string` | ✅ **Compatible** |
| `name: string` | `name: string` | ✅ **Compatible** |
| `description?: string` | `description?: string` | ✅ **Compatible** |
| `repository: Repository` | `repository: Repository` | ⚠️ **Structure differences** |
| `codeExecution: CodeExecution` | `codeExecution: CodeExecution` | ⚠️ **Structure differences** |
| `variables: Record<string, string>` | `variables: Record<string, string>` | ✅ **Compatible** |
| `createdAt: string` | `createdAt: Date` | ✅ String→Date conversion |
| `updatedAt: string` | `updatedAt: Date` | ✅ String→Date conversion |
| `isActive: boolean` | `isActive: boolean` | ✅ **Compatible** |

### Repository Type Differences

**Client Repository Types:**
- `LocalRepository`: `{ type: 'local', path: string, useGitWorktrees?: boolean }`
- `GithubRepository`: `{ type: 'github', repoId: string, branch?: string, commit?: string, token?: string }`

**Shared Repository Types:**
- `LocalRepository`: `{ type: 'local', path: string, useGitWorktrees?: boolean, originalBranch?: string }`
- `GithubRepository`: `{ type: 'github', repoId: string, branch: string, commit?: string, token?: string }`
- **NEW**: `GitlabRepository` support

**Key Differences:**
- ✅ **NEW FIELD**: `originalBranch?: string` in LocalRepository
- ⚠️ **BREAKING**: `branch` is required (not optional) in GithubRepository
- ✅ **NEW TYPE**: GitLab repository support

### CodeExecution Type Differences

**Client:**
```typescript
interface CodeExecution {
  type: 'local' | 'docker'
  setupCommands?: string
  serverCommand?: string
  image?: string
  envVars?: Record<string, string>
  volumes?: Array<string>
  ports?: Record<string, number>
}
```

**Shared:**
```typescript
interface CodeExecution {
  type: 'local' | 'docker' | 'sandbox'  // NEW: 'sandbox' type
  setupCommands?: string
  serverCommand?: string
  serverPortPlaceholder?: string        // NEW FIELD
  serverLogFile?: string               // NEW FIELD
  image?: string
  envVars?: Record<string, string>
  volumes?: string[]                   // Array<string> → string[]
  ports?: Record<string, number>
}
```

**Migration Impact:**
- ✅ **NEW TYPE**: 'sandbox' execution type
- ✅ **NEW FIELDS**: `serverPortPlaceholder`, `serverLogFile`
- ✅ **TYPE CHANGE**: `volumes` array type (cosmetic)

### Response Type Differences

**Client EnvironmentResponse:**
- Uses snake_case: `created_at`, `updated_at`, `is_active`, `code_execution`

**Shared EnvironmentResponse:**
- Uses camelCase consistently
- Same structure as base Environment interface

## Agent Types Analysis

### Agent Request Types

| **Client Type** | **Shared Equivalent** | **Conversion Required** |
|-----------------|----------------------|-------------------------|
| `AgentOptions` | **No direct equivalent** | ⚠️ **Client-specific type** |
| `AgentTriggerRequest` | `CreateTaskRequest` | ✅ **Structure differences** |
| `AgentTriggerResponse` | `SimpleTaskResponse` | ✅ **Structure differences** |

### AgentTriggerRequest vs CreateTaskRequest

**Client AgentTriggerRequest:**
```typescript
interface AgentTriggerRequest {
  agentType: string
  message: string
  environmentId: string
  branch?: string
}
```

**Shared CreateTaskRequest:**
```typescript
interface CreateTaskRequest {
  agentType: string
  environmentId: string
  message: string
  branchName?: string  // Different field name
  start?: boolean      // NEW FIELD with default true
}
```

**Migration Required:**
- ✅ Field rename: `branch` → `branchName`
- ✅ **NEW FIELD**: `start?: boolean` (defaults to true)
- ✅ Field reordering (cosmetic)

## Git Diff Types Analysis

### GitDiff Structure Differences

**Client GitDiffFile:**
```typescript
interface GitDiffFile {
  hunks: Array<Hunk>              // Complex Hunk objects
  oldEndingNewLine: boolean
  newEndingNewLine: boolean
  oldMode: string
  newMode: string
  similarity?: number
  oldRevision: string
  newRevision: string
  oldPath: string
  newPath: string
  isBinary?: boolean
  type: 'add' | 'delete' | 'modify' | 'rename' | 'copy'
}
```

**Shared GitDiffFile:**
```typescript
interface GitDiffFile {
  oldFile?: string     // Simple string paths
  newFile?: string
  hunks: string[]      // Simple string array
}
```

**Key Differences:**
- ⚠️ **BREAKING**: Completely different structure
- ⚠️ **BREAKING**: Client has detailed git metadata, shared has simplified format
- ⚠️ **BREAKING**: Hunk representation differs (complex objects vs strings)

This is the most significant breaking change requiring careful handling.

## Workspace Types Analysis

### New Shared Types (Not in Client)

The shared types introduce comprehensive workspace management:

```typescript
enum WorkspaceStatus {
  NOT_SETUP = 'not_setup',
  SETTING_UP = 'setting_up', 
  IDLE = 'idle',
  RUNNING = 'running',
}

interface LocalWorkspace extends BaseWorkspace {
  type: 'local'
  path: string
  branch?: string
  originalBranch?: string
}

interface DockerWorkspace extends BaseWorkspace {
  type: 'docker'
  containerId: string
  port?: number
  image?: string
}
```

The client has basic workspace status types that partially overlap:
- `WorkspaceStatus` enum is identical
- `WorkspaceStatusResponse` and `ServerStatus` have some overlap with shared types

## Message and Content Types Analysis

### Message Content Structure

**Client has detailed message content types:**
- `TextContent`, `ToolResultContent`, `ToolUseContent`
- `MessageContent` with role, model, usage information
- Complex message hierarchy: `UserMessage`, `AssistantMessage`, `ResultMessage`

**Shared has simplified message content:**
- `MessageContent` with basic fields
- Focus on event-driven message handling

This represents a **fundamental architectural difference** in how messages are structured and handled.

## API Response Types Analysis

### New Shared API Patterns

The shared types introduce modern API patterns not present in client:

```typescript
interface PaginatedResponse<T> {
  items: T[]
  total: number
  page: number
  pageSize: number
  hasMore: boolean
}

interface ApiError {
  code: string
  message: string
  details?: any
}
```

These provide standardized error handling and pagination that the client currently lacks.

## Claude Code Types Analysis

**Client-Only Types:**
- `ClaudeCodeSettingsResponse`
- `UpdateClaudeCodeSettingsRequest`
- `ClaudeCodeToolsResponse`
- `PermissionMode`

These are **client-specific** and have no shared equivalent, representing UI-only functionality.

## Migration Strategy Recommendations

### Phase 1: Direct Replacements (Low Risk)
- ✅ Replace basic utility types and enums where structure is identical
- ✅ Update naming conventions (snake_case → camelCase)
- ✅ Update date handling (string → Date)

### Phase 2: Structural Changes (Medium Risk)
- ⚠️ Update event type handling to support new EventType enum
- ⚠️ Migrate environment response handling
- ⚠️ Update task iteration ID handling (string → number)

### Phase 3: Breaking Changes (High Risk)
- 🚨 **GitDiff types**: Requires complete rework of diff rendering
- 🚨 **Message architecture**: May require rethinking message display
- 🚨 **Repository validation**: Handle new required fields

### Phase 4: Feature Additions (Enhancement)
- ✅ Add support for new workspace types
- ✅ Implement pagination support  
- ✅ Add standardized error handling
- ✅ Support new event types

## Type Compatibility Matrix

| **Type Category** | **Compatibility** | **Risk Level** | **Migration Effort** |
|-------------------|-------------------|----------------|----------------------|
| **Basic Task Types** | 🟡 Partial | Medium | Moderate - naming & validation |
| **Event Types** | 🟡 Partial | Medium | Moderate - enum expansion |
| **Environment Types** | 🟡 Partial | Medium | Moderate - field additions |
| **Agent Types** | 🟡 Partial | Low | Low - minor field changes |
| **GitDiff Types** | 🔴 Incompatible | High | High - complete rework |
| **Message Types** | 🔴 Incompatible | High | High - architectural change |
| **Claude Code Types** | 🟢 Client-Only | Low | Low - keep as-is |
| **Workspace Types** | 🟢 Additive | Low | Low - add new features |

## Conclusion

The migration involves significant structural changes beyond simple naming convention updates. The most challenging aspects will be:

1. **GitDiff handling**: Complete rework required
2. **Message architecture**: Fundamental differences in approach
3. **Field requirement changes**: Several fields change from required to optional or vice versa
4. **New functionality integration**: Workspace management, pagination, enhanced error handling

Success requires careful planning, comprehensive testing, and possibly maintaining compatibility layers during transition.