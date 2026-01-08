# Plan: Worker Event Authentication & Row Level Security

## Problem Statement

Currently, workers emit events via PostgreSQL without proper authentication context:

1. **No user validation on event emission** - Any worker can emit events for any task
2. **Internal bypass methods required** - `getInternal()`, `getTaskOwner()` exist to bypass access control
3. **No session context during event processing** - NotifyListenerService processes events without user context
4. **Worker tokens exist but aren't used for event validation**
5. **No database-level security** - Access control is purely application-level (defense-in-depth missing)

## Current Architecture

```
Worker Pod                          Main API
───────────────────────────────────────────────────────────
   │                                    │
   ├─ Has WORKER_AUTH_TOKEN env var     │
   │  (JWT with userId, taskId, etc.)   │
   │                                    │
   ├─ EventService.emitEvent(event)     │
   │  ↓                                 │
   │  PostgreSQL INSERT (pending)       │
   │  ← NO user context set             │
   │  ← NO RLS enforcement              │
   │  pg_notify('task_events', ...)     │
   │                                    ↓
   │                          NotifyListenerService
   │                          receives NOTIFY
   │                                    │
   │                          NO user context ❌
   │                          NO event validation ❌
   │                                    │
   │                          eventService.emitEvent()
   │                          → SSE broadcast
```

## Target Architecture

```
Worker Pod                          Main API
───────────────────────────────────────────────────────────
   │                                    │
   ├─ Has WORKER_AUTH_TOKEN env var     │
   │  (JWT with userId, taskId, etc.)   │
   │                                    │
   ├─ Connects as 'worker_user' role ✓  │  ← Connects as 'app_user' role
   │  (can ONLY INSERT task_events)     │  (full access with RLS)
   │                                    │
   ├─ SET LOCAL app.user_id = '...'  ✓  │
   │  (PostgreSQL session variable)     │
   │                                    │
   ├─ EventService.emitEvent(event)     │
   │  ↓                                 │
   │  PostgreSQL INSERT:                │
   │   - event payload                  │
   │   - user_id from session var ✓     │
   │   - RLS policy enforces user ✓     │
   │   - Role restricts to INSERT ✓    │
   │                                    │
   │  pg_notify('task_events', ...)     │
   │                                    ↓
   │                          NotifyListenerService
   │                          receives NOTIFY
   │                                    │
   │                          1. Validate auth_token ✓
   │                          2. SessionContext.run() ✓
   │                          3. SET LOCAL app.user_id ✓
   │                                    │
   │                          RLS + application access control
   │                          (defense-in-depth)
```

### Security Layers

| Layer | Main API | Worker Pod |
|-------|----------|------------|
| **PostgreSQL Role** | `app_user` (full CRUD) | `worker_user` (INSERT events only) |
| **RLS Policies** | Filter by `app.user_id` | Filter by `app.user_id` |
| **Application Auth** | Session/API Key | JWT Worker Token |
| **Table Access** | All tables | `task_events` only |

## Implementation Steps

### Phase 0: PostgreSQL Row Level Security (RLS)

RLS provides database-level enforcement of access control. Even if application code has bugs, PostgreSQL will not return unauthorized rows.

#### 0.1 Database Session Context Setup

PostgreSQL session variables are used to pass user identity to RLS policies.

**File:** `apps/backend/src/infrastructure/database/session-context-db.ts` (NEW)

```typescript
import type { Sql } from 'postgres'

/**
 * Set PostgreSQL session variables for RLS policies.
 * Must be called within a transaction or at connection setup.
 *
 * Uses SET LOCAL so variables are scoped to current transaction only.
 */
export async function setDatabaseUserContext(
  sql: Sql,
  userId: string | null,
  groupId?: string | null,
): Promise<void> {
  // SET LOCAL ensures the setting is transaction-scoped
  // If userId is null, set to empty string (RLS policies check for this)
  await sql`SELECT set_config('app.user_id', ${userId || ''}, true)`
  await sql`SELECT set_config('app.group_id', ${groupId || ''}, true)`
}

/**
 * Clear database user context (for cleanup/testing).
 */
export async function clearDatabaseUserContext(sql: Sql): Promise<void> {
  await sql`SELECT set_config('app.user_id', '', true)`
  await sql`SELECT set_config('app.group_id', '', true)`
}
```

#### 0.2 RLS Policies Migration

**File:** `apps/backend/drizzle-postgres/XXXX_add_rls_policies.sql`

```sql
-- Enable RLS on tables
ALTER TABLE tasks ENABLE ROW LEVEL SECURITY;
ALTER TABLE task_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE repositories ENABLE ROW LEVEL SECURITY;

-- =============================================================================
-- DATABASE ROLES
-- =============================================================================

-- Main app role (full access, RLS still applies)
DO $$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'app_user') THEN
    CREATE ROLE app_user NOLOGIN;
  END IF;
END
$$;

-- Worker role (HIGHLY RESTRICTED - can only INSERT events)
DO $$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'worker_user') THEN
    CREATE ROLE worker_user NOLOGIN;
  END IF;
END
$$;

-- =============================================================================
-- PERMISSIONS FOR app_user (Main API)
-- =============================================================================
GRANT SELECT, INSERT, UPDATE, DELETE ON tasks TO app_user;
GRANT SELECT, INSERT, UPDATE, DELETE ON task_events TO app_user;
GRANT SELECT, INSERT, UPDATE, DELETE ON repositories TO app_user;
GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO app_user;

-- =============================================================================
-- PERMISSIONS FOR worker_user (Worker Pods) - MINIMAL PRIVILEGES
-- =============================================================================
-- Workers can ONLY:
-- 1. INSERT into task_events (emit events)
-- 2. SELECT from task_events for sequence/ID generation
-- They CANNOT: read tasks, read repositories, update anything, delete anything

GRANT INSERT ON task_events TO worker_user;
GRANT SELECT ON task_events TO worker_user;  -- Needed for RETURNING clause
GRANT USAGE ON SEQUENCE task_events_id_seq TO worker_user;

-- Explicitly REVOKE everything else (belt and suspenders)
REVOKE ALL ON tasks FROM worker_user;
REVOKE ALL ON repositories FROM worker_user;
REVOKE ALL ON sessions FROM worker_user;
REVOKE ALL ON api_keys FROM worker_user;

-- =============================================================================
-- RLS POLICIES FOR TASKS
-- =============================================================================

-- Policy: Users can see their own tasks, public tasks, or legacy tasks (null owner)
CREATE POLICY tasks_select_policy ON tasks
  FOR SELECT
  USING (
    created_by IS NULL                                    -- Legacy (no owner)
    OR created_by = current_setting('app.user_id', true)  -- Owner
    OR visibility = 'public'                              -- Public visibility
  );

-- Policy: Users can only insert tasks they own
CREATE POLICY tasks_insert_policy ON tasks
  FOR INSERT
  WITH CHECK (
    created_by IS NULL                                    -- Legacy/system
    OR created_by = current_setting('app.user_id', true)  -- Must be owner
  );

-- Policy: Users can only update their own tasks or legacy tasks
CREATE POLICY tasks_update_policy ON tasks
  FOR UPDATE
  USING (
    created_by IS NULL
    OR created_by = current_setting('app.user_id', true)
  );

-- Policy: Users can only delete their own tasks
CREATE POLICY tasks_delete_policy ON tasks
  FOR DELETE
  USING (
    created_by = current_setting('app.user_id', true)
  );

-- =============================================================================
-- RLS POLICIES FOR TASK_EVENTS
-- =============================================================================

-- Policy: Users can see events for tasks they can access
-- (References tasks table visibility through subquery)
CREATE POLICY task_events_select_policy ON task_events
  FOR SELECT
  USING (
    user_id IS NULL                                       -- Legacy events
    OR user_id = current_setting('app.user_id', true)     -- User's own events
    OR EXISTS (                                           -- Events for accessible tasks
      SELECT 1 FROM tasks t
      WHERE t.task_id = task_events.task_id
      AND (
        t.created_by IS NULL
        OR t.created_by = current_setting('app.user_id', true)
        OR t.visibility = 'public'
      )
    )
  );

-- Policy: Users can only insert events for their own tasks
CREATE POLICY task_events_insert_policy ON task_events
  FOR INSERT
  WITH CHECK (
    user_id IS NULL                                       -- Legacy/system
    OR user_id = current_setting('app.user_id', true)     -- Must be event owner
  );

-- Policy: System can update (mark delivered) any event
CREATE POLICY task_events_update_policy ON task_events
  FOR UPDATE
  USING (true);  -- System operation, always allowed

-- =============================================================================
-- RLS POLICIES FOR REPOSITORIES
-- =============================================================================

-- Policy: Users can see their own repos, public repos, or legacy repos
CREATE POLICY repositories_select_policy ON repositories
  FOR SELECT
  USING (
    created_by IS NULL
    OR created_by = current_setting('app.user_id', true)
    OR visibility = 'public'
  );

-- Policy: Users can only insert repos they own
CREATE POLICY repositories_insert_policy ON repositories
  FOR INSERT
  WITH CHECK (
    created_by IS NULL
    OR created_by = current_setting('app.user_id', true)
  );

-- Policy: Users can only update their own repos or legacy repos
CREATE POLICY repositories_update_policy ON repositories
  FOR UPDATE
  USING (
    created_by IS NULL
    OR created_by = current_setting('app.user_id', true)
  );

-- Policy: Users can only delete their own repos
CREATE POLICY repositories_delete_policy ON repositories
  FOR DELETE
  USING (
    created_by = current_setting('app.user_id', true)
  );
```

#### 0.3 Integration with Database Layer

**File:** `apps/backend/src/infrastructure/database/postgres.ts` (MODIFY)

```typescript
import { setDatabaseUserContext } from './session-context-db'

export class PostgresDatabase implements Database {
  // ... existing code

  /**
   * Execute queries within a user context (for RLS).
   * Sets PostgreSQL session variables before running the function.
   */
  async withUserContext<T>(
    userId: string | null,
    groupId: string | null,
    fn: () => Promise<T>,
  ): Promise<T> {
    return await this.transaction(async (tx) => {
      await setDatabaseUserContext(this.client, userId, groupId)
      return await fn()
    })
  }
}
```

#### 0.4 Middleware Integration

**File:** `apps/backend/src/middleware/database-context.middleware.ts` (NEW)

```typescript
import type { Context, Next } from 'hono'
import { appContainer } from '../container/container'
import { Database } from '../infrastructure/database/database'
import { setDatabaseUserContext } from '../infrastructure/database/session-context-db'
import { getSessionFromContext } from './auth.middleware'

/**
 * Middleware that sets PostgreSQL session variables from authenticated session.
 * Must run AFTER auth middleware.
 */
export const databaseContext = async (c: Context, next: Next) => {
  const session = getSessionFromContext(c)

  if (session) {
    const db = appContainer.get(Database)
    const rawClient = db.getRawClient()

    // Set user context for RLS policies
    await setDatabaseUserContext(rawClient, session.userId, session.groupId || null)
  }

  await next()
}
```

### Phase 1: Database Schema Changes

#### 1.1 Add authentication columns to task_events table

```sql
-- Migration: add_auth_columns_to_task_events.sql
ALTER TABLE task_events ADD COLUMN user_id VARCHAR;
ALTER TABLE task_events ADD COLUMN auth_token TEXT;

-- Index for user-scoped queries
CREATE INDEX idx_task_events_user_id ON task_events(user_id);

-- Backfill user_id from task ownership for existing events
UPDATE task_events te
SET user_id = t.created_by
FROM tasks t
WHERE te.task_id = t.task_id
AND te.user_id IS NULL
AND t.created_by IS NOT NULL;
```

**Files to modify:**
- `apps/backend/drizzle-postgres/` - New migration file
- `apps/backend/src/infrastructure/database/schema.ts` - Add columns to schema

#### 1.2 Update EventRepository

**File:** `apps/backend/src/features/events/db/event.repository.ts`

```typescript
interface SaveEventOptions {
  event: TaskEvent
  userId?: string      // NEW: User who owns this event
  authToken?: string   // NEW: JWT for validation
}

async savePendingEvent(options: SaveEventOptions): Promise<void> {
  const { event, userId, authToken } = options
  await this.db.drizzle.insert(taskEvents).values({
    // ... existing fields
    userId,      // NEW
    authToken,   // NEW
  })
}
```

### Phase 2: Worker Event Service Enhancement

#### 2.1 Create WorkerEventService with auth context

**File:** `apps/backend/src/features/events/services/worker-event.service.ts` (NEW)

```typescript
/**
 * Event service for worker mode that includes authentication context.
 * Extends base EventService with user identity from worker token.
 */
@injectable()
export class WorkerEventService {
  constructor(
    @inject(EventService) private eventService: EventService,
    @inject(WorkerTokenService) private tokenService: WorkerTokenService,
  ) {}

  /**
   * Emit event with authenticated user context.
   * User identity extracted from WORKER_AUTH_TOKEN env var.
   */
  async emitEvent(event: TaskEvent): Promise<void> {
    const authToken = process.env['WORKER_AUTH_TOKEN']
    if (!authToken) {
      throw new Error('WORKER_AUTH_TOKEN required for event emission')
    }

    // Verify token and extract user info
    const payload = await this.tokenService.verifyToken(authToken)
    if (!payload) {
      throw new Error('Invalid worker auth token')
    }

    // Validate event belongs to this worker's task
    if (event.taskId !== payload.taskId) {
      throw new Error(`Worker not authorized to emit events for task ${event.taskId}`)
    }

    // Emit with user context
    await this.eventService.emitEventWithAuth({
      event,
      userId: payload.userId,
      authToken,
    })
  }
}
```

#### 2.2 Worker Database Connection (Restricted Role)

Workers must connect with a restricted PostgreSQL role that can only INSERT events.

**Environment Variables:**

```bash
# Main API uses app_user role (full access)
DATABASE_URL=postgresql://app_user:password@host:5432/moatless

# Worker uses worker_user role (INSERT events only)
WORKER_DATABASE_URL=postgresql://worker_user:password@host:5432/moatless
```

**File:** `apps/backend/src/worker-app.ts`

```typescript
import { setDatabaseUserContext } from './infrastructure/database/session-context-db'

// Worker uses WORKER_DATABASE_URL (restricted role)
const workerDbUrl = process.env['WORKER_DATABASE_URL'] || process.env['DATABASE_URL']
if (!workerDbUrl) {
  throw new Error('WORKER_DATABASE_URL or DATABASE_URL required')
}

// Create database connection with restricted worker_user role
const database = createPostgresDatabase(workerDbUrl)

// Validate worker token and extract user identity
const authToken = process.env['WORKER_AUTH_TOKEN']
if (!authToken) {
  throw new Error('WORKER_AUTH_TOKEN required')
}
const tokenPayload = await workerTokenService.verifyToken(authToken)
if (!tokenPayload) {
  throw new Error('Invalid WORKER_AUTH_TOKEN')
}

// Set PostgreSQL user context for RLS enforcement
// This determines WHICH user's events can be inserted
const rawClient = database.getRawClient()
await setDatabaseUserContext(rawClient, tokenPayload.userId, tokenPayload.groupId || null)

// Replace EventService with WorkerEventService
const workerEventService = new WorkerEventService(eventService, workerTokenService)

// Use in agent execution
for await (const event of agent.run(message, runOptions)) {
  await workerEventService.emitEvent(event)  // Now validates & includes auth
}

// Use in PreviewServer lifecycle sink
class WorkerPreviewLifecycleSink implements PreviewServerLifecycleSink {
  constructor(private readonly eventService: WorkerEventService) {}
  // ...
}
```

**Security guarantees:**

1. **Role restriction**: Worker connects as `worker_user` which can ONLY INSERT into `task_events`
   - Attempting `SELECT * FROM tasks` → Permission denied
   - Attempting `DELETE FROM task_events` → Permission denied
   - Attempting `INSERT INTO repositories` → Permission denied

2. **RLS enforcement**: Even for allowed INSERT, RLS ensures `user_id` matches `app.user_id`
   - Worker for User A cannot insert events claiming to be User B

3. **Defense-in-depth**: Three layers must all pass:
   - PostgreSQL role allows INSERT on task_events ✓
   - RLS policy allows INSERT for this user_id ✓
   - Application validates JWT token matches event ✓

#### 2.3 Kubernetes Pod Configuration

**File:** `k8s/worker-deployment.yaml` (example)

```yaml
env:
  # Restricted database URL with worker_user role
  - name: WORKER_DATABASE_URL
    valueFrom:
      secretKeyRef:
        name: worker-db-credentials
        key: url

  # JWT token with user identity (generated by main app)
  - name: WORKER_AUTH_TOKEN
    valueFrom:
      secretKeyRef:
        name: worker-{{ .taskId }}-auth
        key: token

  - name: WORKER_TASK_ID
    value: "{{ .taskId }}"
```

### Phase 3: NotifyListenerService Authentication

#### 3.1 Validate events and establish session context

**File:** `apps/backend/src/features/events/services/notify-listener.service.ts`

```typescript
async handleNotification(payload: NotifyPayload): Promise<void> {
  const event = await this.eventRepository.getEventById(payload.eventId)
  if (!event) return

  // NEW: Validate auth token if present
  if (event.authToken) {
    const tokenPayload = await this.workerTokenService.verifyToken(event.authToken)
    if (!tokenPayload) {
      logger.warn({ eventId: event.eventId }, 'Invalid auth token on event, skipping')
      return
    }

    // Verify user_id matches token
    if (event.userId && event.userId !== tokenPayload.userId) {
      logger.warn({ eventId: event.eventId }, 'User ID mismatch, skipping')
      return
    }

    // NEW: Create synthetic session from token
    const session: UserSession = {
      id: `worker-${tokenPayload.taskId}`,
      provider: 'worker-token',
      userId: tokenPayload.userId,
      userLogin: tokenPayload.userLogin,
      userEmail: tokenPayload.userEmail || null,
      groupId: tokenPayload.groupId,
      accessToken: '',
      createdAt: new Date(),
      expiresAt: new Date(Date.now() + 24 * 60 * 60 * 1000),
    }

    // NEW: Process event within session context
    await SessionContext.run(session, async () => {
      await this.eventService.emitEvent(event)
      await this.eventRepository.markAsDelivered(event.eventId)
    })
  } else {
    // Legacy events without auth - process as before
    await this.eventService.emitEvent(event)
    await this.eventRepository.markAsDelivered(event.eventId)
  }
}
```

### Phase 4: Remove Internal Bypass Methods

#### 4.1 Audit and remove getInternal() usage

**Files to check:**
- `apps/backend/src/features/tasks/db/task.repository.ts`
- `apps/backend/src/features/tasks/services/task.service.ts`

**Current internal methods to deprecate:**
```typescript
// TaskRepository
async getInternal(taskId: string): Promise<Task | null>  // Remove
async listAll(): Promise<Task[]>                          // Remove

// TaskService
async getTaskOwner(taskId: string): Promise<string | undefined>  // Remove
```

**Replacement pattern:**
- With session context established from worker token, use standard `get()` and `list()` methods
- Access control naturally applies via `withVisibilityScope()`

#### 4.2 Update callers to use session context

**Example transformation:**

Before:
```typescript
// MCP tool execution - bypass access control
const owner = await this.taskService.getTaskOwner(taskId)
// ... use owner for something
```

After:
```typescript
// Session already established from worker token
const task = await this.taskService.getTask(taskId)  // Standard method with access control
// task.createdBy is available if user has access
```

### Phase 5: Security Hardening

#### 5.1 Event validation on storage

**File:** `apps/backend/src/features/events/services/event.service.ts`

```typescript
async emitEventWithAuth(options: {
  event: TaskEvent
  userId: string
  authToken: string
}): Promise<void> {
  const { event, userId, authToken } = options

  // Verify token before saving
  const payload = await this.tokenService.verifyToken(authToken)
  if (!payload || payload.userId !== userId) {
    throw new Error('Invalid authentication for event emission')
  }

  // Save with auth context
  await this.eventRepository.savePendingEvent({
    event,
    userId,
    authToken,
  })
}
```

#### 5.2 Rate limiting on event emission

Consider adding rate limiting to prevent event flooding:

```typescript
// Per-task rate limit
const EVENTS_PER_MINUTE = 100

async emitEventWithAuth(options: EmitOptions): Promise<void> {
  const key = `event-rate:${options.event.taskId}`
  const count = await this.rateLimiter.increment(key, 60)
  if (count > EVENTS_PER_MINUTE) {
    throw new Error('Event rate limit exceeded')
  }
  // ... continue
}
```

## Migration Strategy

### Backward Compatibility

1. **Auth columns are nullable** - Existing events continue to work
2. **Legacy event processing** - Events without authToken processed as before
3. **Gradual rollout** - New workers emit authenticated events, old events drain naturally

### Rollout Order

1. Deploy database migration (add columns)
2. Deploy main API with NotifyListenerService changes
3. Deploy workers with WorkerEventService
4. Monitor for auth failures in logs
5. After all workers updated, consider making authToken required

## Benefits

1. **Defense-in-depth** - Database-level RLS + application-level validation
2. **Security** - Only authorized workers can emit events for their assigned tasks
3. **Audit trail** - Events have clear user attribution
4. **Simplified code** - Remove internal bypass methods and application-level access filters
5. **Consistent access control** - Same rules apply to API requests and worker events
6. **Future-proof** - RLS policies are centralized in SQL, not scattered across repositories

## Files to Modify

| File | Changes |
|------|---------|
| **Database & RLS** | |
| `drizzle-postgres/XXXX_add_rls_policies.sql` | NEW - RLS policies, roles (app_user, worker_user) |
| `infrastructure/database/session-context-db.ts` | NEW - PostgreSQL session variable helpers |
| `infrastructure/database/postgres.ts` | Add withUserContext() method |
| `infrastructure/database/schema.ts` | Add userId, authToken columns to task_events |
| **Middleware** | |
| `middleware/database-context.middleware.ts` | NEW - Set DB user context from session |
| **Events Feature** | |
| `features/events/db/event.repository.ts` | Update save methods with userId |
| `features/events/services/event.service.ts` | Add emitEventWithAuth() |
| `features/events/services/worker-event.service.ts` | NEW - Worker-specific event service |
| `features/events/services/notify-listener.service.ts` | Session context + DB context establishment |
| **Worker** | |
| `worker-app.ts` | Use WORKER_DATABASE_URL, set DB user context |
| **Cleanup (RLS replaces app-level filters)** | |
| `features/tasks/db/task.repository.ts` | Remove getInternal(), withVisibilityScope |
| `features/tasks/services/task.service.ts` | Remove getTaskOwner() |
| `infrastructure/database/access-control.ts` | Simplify or remove |
| **Configuration** | |
| `k8s/worker-deployment.yaml` | Add WORKER_DATABASE_URL env var |
| `.env.example` | Document WORKER_DATABASE_URL |
| `features/workspace/provisioners/kubernetes-workspace-provisioner.ts` | Pass WORKER_DATABASE_URL to pods |

## Testing Plan

### Unit Tests
- WorkerEventService.emitEvent() validates token
- WorkerEventService.emitEvent() rejects wrong taskId
- NotifyListenerService establishes session context
- NotifyListenerService rejects invalid tokens
- setDatabaseUserContext() sets correct session variables

### Integration Tests
- Worker → PostgreSQL → NotifyListener → SSE flow with auth
- Rate limiting enforcement
- Legacy event processing (no authToken)
- RLS: User A cannot read User B's tasks
- RLS: User A cannot insert events for User B's tasks
- RLS: Public tasks visible to all users
- RLS: Legacy (null owner) records visible to all

### E2E Tests
- Create task, start agent, verify events have userId
- Verify access control applies during event processing
- Worker pod with user A token cannot emit events for user B task

### RLS Specific Tests

```sql
-- Test RLS policies in psql:

-- 1. Set user context
SELECT set_config('app.user_id', 'user-alice', false);

-- 2. Verify only Alice's tasks visible
SELECT task_id, created_by FROM tasks;  -- Should only show Alice's + public + legacy

-- 3. Switch to Bob
SELECT set_config('app.user_id', 'user-bob', false);

-- 4. Verify Bob cannot see Alice's private tasks
SELECT task_id, created_by FROM tasks;  -- Should NOT include Alice's private tasks

-- 5. Verify Bob cannot insert event for Alice's task
INSERT INTO task_events (event_id, task_id, type, user_id, ...)
VALUES ('evt-1', 'alice-task-id', 'agent', 'user-bob', ...);
-- Should FAIL with RLS policy violation
```

### Worker Role Restriction Tests

```sql
-- Test worker_user role restrictions in psql:

-- 1. Connect as worker_user
SET ROLE worker_user;
SELECT set_config('app.user_id', 'user-alice', false);

-- 2. Verify INSERT works on task_events
INSERT INTO task_events (event_id, task_id, type, user_id, timestamp)
VALUES ('evt-test', 'alice-task', 'agent', 'user-alice', NOW());
-- Should SUCCEED

-- 3. Verify SELECT on other tables FAILS
SELECT * FROM tasks;
-- ERROR: permission denied for table tasks

-- 4. Verify DELETE FAILS
DELETE FROM task_events WHERE event_id = 'evt-test';
-- ERROR: permission denied for table task_events

-- 5. Verify INSERT on other tables FAILS
INSERT INTO tasks (task_id, ...) VALUES ('hacked', ...);
-- ERROR: permission denied for table tasks

-- 6. Verify INSERT with wrong user_id FAILS (RLS)
INSERT INTO task_events (event_id, task_id, type, user_id, timestamp)
VALUES ('evt-evil', 'bob-task', 'agent', 'user-bob', NOW());
-- ERROR: new row violates row-level security policy
```

## Considerations

### RLS Performance

RLS policies add overhead to every query. Mitigation:
- Index on `created_by` column
- Index on `visibility` column
- Test with representative data volumes

### Connection Pooling

Session variables are connection-scoped. With connection pooling:
- `SET LOCAL` (within transaction) is preferred
- Alternative: Reset context after each request
- Consider: Use separate connections per user (expensive)

### Superuser Access

RLS policies don't apply to superusers. For admin operations:
- Use a non-superuser role for the application
- Or bypass RLS explicitly: `SET ROLE app_user`

### Migration Path

1. Deploy RLS policies with permissive defaults
2. Deploy application changes to set user context
3. Test thoroughly in staging
4. Tighten policies gradually
