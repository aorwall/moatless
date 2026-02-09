# Plan: Drop Unused Columns from task_messages

## Context

The `task_messages` table carries 12 columns that are **never read** anywhere in the Rust backend and are always written as `None`/NULL. These originated from the Claude Code SDK message envelope and Claude API response fields, but the Rust backend only uses a subset. Dropping them simplifies the model and removes dead weight from the DB.

**`run_metrics` vs `assistant_usage_*`**: These are NOT duplicates — `run_metrics` is per-turn aggregate (on result messages) with cost info, while `assistant_usage_*` is per-message token counts from the Claude API (on assistant messages). Both are actively used.

## Columns to Drop

| Column | Why it's dead |
|--------|--------------|
| `parent_uuid` | Always NULL in Rust; never read |
| `is_sidechain` | Always NULL; never read |
| `cwd` | Always NULL; never read |
| `version` | Always NULL; never read |
| `git_branch` | Always NULL in messages; never read |
| `request_id` | Always NULL in Rust; never read |
| `assistant_message_type` | Always "message" literal; never read |
| `assistant_stop_sequence` | Never read in Rust or frontend |
| `tool_use_result` | Always NULL in Rust; never read |
| `attachments` | Always NULL in Rust; never read |

**Keeping** (still actively used):
- `user_type` — NOT NULL constraint, meaningful values ("human"/"system"), might be useful for queries
- `created_at` — Auto-populated DB metadata, harmless

## Changes

### 1. DB migration: Drop columns
**New file**: `backend/migrations/0002_drop_unused_message_columns.sql`

```sql
ALTER TABLE task_messages DROP COLUMN IF EXISTS parent_uuid;
ALTER TABLE task_messages DROP COLUMN IF EXISTS is_sidechain;
ALTER TABLE task_messages DROP COLUMN IF EXISTS cwd;
ALTER TABLE task_messages DROP COLUMN IF EXISTS version;
ALTER TABLE task_messages DROP COLUMN IF EXISTS git_branch;
ALTER TABLE task_messages DROP COLUMN IF EXISTS request_id;
ALTER TABLE task_messages DROP COLUMN IF EXISTS assistant_message_type;
ALTER TABLE task_messages DROP COLUMN IF EXISTS assistant_stop_sequence;
ALTER TABLE task_messages DROP COLUMN IF EXISTS tool_use_result;
ALTER TABLE task_messages DROP COLUMN IF EXISTS attachments;
```

Also add a Drizzle migration: `drizzle/0020_drop_unused_message_columns.sql` (same SQL).

### 2. Update consolidated test schema
**File**: `backend/migrations/0001_schema.sql`

Remove the 10 columns from the `CREATE TABLE task_messages` definition.

### 3. Update Rust model
**File**: `backend/src/db/models.rs`

Remove from `TaskMessageRow`:
- `parent_uuid`, `is_sidechain`, `cwd`, `version`, `git_branch`, `request_id`
- `assistant_message_type`, `assistant_stop_sequence`
- `tool_use_result`, `attachments`

### 4. Update Rust SQL queries
**File**: `backend/src/db/message_dao.rs`

- **SELECT queries** (`get_all`, `get_pending_user`, `get_last_result`): Remove the 10 columns from SELECT lists
- **INSERT** (`append`): Remove the 10 columns from INSERT column list and VALUES binds

### 5. Update Rust message creation
**File**: `backend/src/agent/orchestration.rs`

Remove the 10 fields from `TaskMessageRow` construction in `send_message_flow` and `send_tool_response_flow`.

**File**: `backend/src/event_handler/agent_handlers.rs`

Remove the 10 fields from `build_result_message_row`.

### 6. Update Node.js Drizzle schema
**File**: `apps/backend/src/features/tasks/db/task-message.schema.ts`

Remove the 10 column definitions from the `taskMessages` pgTable. Also remove references in:
- `agentMessageToInsert()` — remove setting these fields
- `applyCommonRowFields()` — remove `parentUuid`, `isSidechain`, `cwd`
- `applyUserInsertFields()` — remove `toolUseResult`
- `applySystemInsertFields()` — remove `toolUseResult`
- `applyAssistantMessageMetadata()` — remove `assistantMessageType`, `assistantStopSequence`
- `selectToAgentMessage()` helper functions — remove reads of dropped columns

### 7. Update Node.js types
**File**: `packages/types/src/schemas/agent-message.schemas.ts` (or similar)

Remove `parentUuid`, `isSidechain`, `cwd`, `toolUseResult`, `attachments` from AgentMessage types if they exist there.

## Files to Modify
1. `backend/migrations/0001_schema.sql` — Remove cols from test schema
2. `backend/migrations/0002_drop_unused_message_columns.sql` — **NEW** migration
3. `drizzle/0020_drop_unused_message_columns.sql` — **NEW** Drizzle migration
4. `backend/src/db/models.rs` — Remove 10 fields from `TaskMessageRow`
5. `backend/src/db/message_dao.rs` — Update SELECT/INSERT queries
6. `backend/src/agent/orchestration.rs` — Remove fields from struct construction
7. `backend/src/event_handler/agent_handlers.rs` — Remove fields from `build_result_message_row`
8. `apps/backend/src/features/tasks/db/task-message.schema.ts` — Remove from Drizzle schema + helpers
9. `packages/types/src/schemas/agent-message.schemas.ts` — Remove from type definitions

## Verification
1. `cargo check` + `cargo test` in `backend/`
2. `bun run type:check` — catch TypeScript breakage from removed fields
3. `bun run biome:check` — formatting
4. Run migration against dev DB
5. Verify frontend still renders messages (no field used by `transform.rs` was removed)
