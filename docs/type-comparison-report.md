# Type System Comparison Report: Client vs Shared

## Overview
This report compares the type definitions between `@client/src/types/` and `@shared/types/` to identify migration requirements and inconsistencies.

## Key Findings

### 1. Naming Convention Differences

#### Snake_case vs CamelCase
The most significant difference is the inconsistent use of naming conventions:

**Client Types (snake_case in API responses):**
- `event_id`, `task_id`, `is_error`
- `code_execution`, `created_at`, `updated_at`
- `is_active`, `total_cost_usd`, `num_turns`

**Shared Types (camelCase):**
- `eventId`, `taskId`, `isError`
- `codeExecution`, `createdAt`, `updatedAt`
- `isActive`, `totalCostUsd`, `numTurns`

### 2. Type Structure Differences

#### Date Types
- **Client**: Uses `string` for dates
- **Shared**: Uses `Date` type
- **Impact**: Need conversion when migrating

#### Environment Types
Client has:
- `EnvironmentResponse` with snake_case fields
- Separate `Environment` interface with camelCase
- `UpdateEnvironmentRequest` uses mixed conventions

Shared has:
- Consistent camelCase throughout
- `EnvironmentResponse` extends `Environment`
- Better type hierarchy

#### Event Types
**Client `event.ts`:**
- Simple `AgentEventType` enum with 4 values
- Single `Event` interface with mixed naming

**Shared `event.ts`:**
- Comprehensive `EventType` enum with 11 values
- `MessageSubType` enum
- Type hierarchy with `BaseEvent` and specific event types
- Better structured with specific event interfaces

#### Task Types
**Client `task.ts`:**
- Complex message system with `MessageType`, `Role`, `ContentType`
- `ClaudeMessage` union type (UserMessage | AssistantMessage | ResultMessage)
- `TaskIteration` with `iteration_id` (snake_case)
- `TaskMetadata` with snake_case fields
- Separate `Task` interface for list view

**Shared `task.ts`:**
- Simplified task structure
- No Claude-specific message types
- `TaskIteration` with `iterationNumber` (camelCase)
- `TaskResponse` and `TaskIterationResponse` for API responses
- Better separation of concerns

### 3. Missing Types in Shared

#### From `agent.ts`:
- `AgentOptions`
- `AgentTriggerRequest`
- `AgentTriggerResponse`

#### From `claude-code.ts`:
- All Claude Code specific types:
  - `PermissionMode`
  - `ClaudeCodeToolInfo`
  - `ClaudeCodeSettingsResponse`
  - `UpdateClaudeCodeSettingsRequest`
  - `ClaudeCodeToolsResponse`

#### From `git-diff.ts`:
- `Hunk` interface with detailed properties
- Client has more detailed git diff structure

#### From `task.ts`:
- Claude-specific message types:
  - `MessageType`, `Role`, `ContentType`
  - `TextContent`, `ToolResultContent`, `ToolUseContent`
  - `Usage` interface
  - `MessageContent`
  - `UserMessage`, `AssistantMessage`, `ResultMessage`
  - `ClaudeMessage` union type

### 4. API Response Types

#### Client Approach:
- Separate interfaces for requests and responses
- Snake_case for API responses
- CamelCase for internal usage

#### Shared Approach:
- Consistent camelCase throughout
- Zod schemas for validation
- Better type safety with inferred types

### 5. Additional Differences

#### Environment Types:
- Client: `GithubRepository.repoId` format: "owner/repo"
- Shared: `GithubRepository.repoId` (same field but no format specification)
- Client: `gitlab` type not supported
- Shared: Supports `gitlab` repository type

#### File Search:
- Client: `FileSearchResult` with `score` and `absolute_path`
- Shared: `FileSearchResponse` with simplified structure

#### Workspace Types:
- Client: `WorkspaceStatusResponse` with `hasUncommittedChanges`
- Shared: More comprehensive workspace types with Docker support

## Migration Requirements

### 1. API Response Transformation
Need to create transformation functions for API responses:
```typescript
// Convert snake_case API responses to camelCase
function transformEnvironmentResponse(apiResponse: any): Environment {
  return {
    id: apiResponse.id,
    createdAt: new Date(apiResponse.created_at),
    updatedAt: new Date(apiResponse.updated_at),
    codeExecution: apiResponse.code_execution,
    isActive: apiResponse.is_active,
    // ... etc
  }
}
```

### 2. Date Conversion
All date strings need to be converted to Date objects when moving from client to shared types.

### 3. Missing Type Migration
For types that don't exist in shared:
- **Agent types**: Create in shared or keep in client-specific directory
- **Claude Code types**: Keep in client-specific directory (UI-only)
- **Claude Message types**: Need to decide if these belong in shared or remain client-specific

### 4. Git Diff Types
The client has more detailed git diff types. Either:
- Migrate the detailed types to shared
- Simplify client usage to match shared

### 5. Event System Alignment
The shared event system is more comprehensive. Client code needs to:
- Adopt the expanded EventType enum
- Use the type hierarchy (BaseEvent, specific events)
- Update event handling code

## Recommendations

1. **Create API transformation layer**: Build a set of transformation functions to convert between API responses (snake_case) and internal types (camelCase).

2. **Gradual migration**: Start with shared types that have direct equivalents:
   - Environment types
   - Basic task types
   - Workspace types

3. **Keep client-specific types**: Maintain Claude Code and Claude Message types in the client as they're UI-specific.

4. **Standardize naming**: Use camelCase consistently in the codebase, transform at API boundaries.

5. **Type safety**: Leverage Zod schemas from shared types for runtime validation.

6. **Consider backwards compatibility**: Some client code may depend on the current type structure, so migration should be careful not to break existing functionality.