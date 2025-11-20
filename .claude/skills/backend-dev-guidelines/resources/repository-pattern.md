# Repository Pattern

## Overview
Repositories abstract database access, providing a clean interface for data operations.

## Basic Repository

```tsx
import { injectable, inject } from 'inversify'
import { TYPES } from '../types'
import { eq } from 'drizzle-orm'
import { tasks } from '../schema'

export interface ITaskRepository {
  findAll(): Promise<Task[]>
  findById(id: string): Promise<Task | null>
  create(data: NewTask): Promise<Task>
  update(id: string, data: Partial<Task>): Promise<Task>
  delete(id: string): Promise<void>
}

@injectable()
export class TaskRepository implements ITaskRepository {
  constructor(@inject(TYPES.Database) private db: Database) {}

  async findAll(): Promise<Task[]> {
    return this.db.select().from(tasks)
  }

  async findById(id: string): Promise<Task | null> {
    const result = await this.db
      .select()
      .from(tasks)
      .where(eq(tasks.id, id))
      .limit(1)
    return result[0] || null
  }

  async create(data: NewTask): Promise<Task> {
    const [task] = await this.db
      .insert(tasks)
      .values(data)
      .returning()
    return task
  }

  async update(id: string, data: Partial<Task>): Promise<Task> {
    const [task] = await this.db
      .update(tasks)
      .set(data)
      .where(eq(tasks.id, id))
      .returning()
    return task
  }

  async delete(id: string): Promise<void> {
    await this.db
      .delete(tasks)
      .where(eq(tasks.id, id))
  }
}
```

## Query Methods

```tsx
@injectable()
export class TaskRepository implements ITaskRepository {
  // ... basic methods

  async findByStatus(status: TaskStatus): Promise<Task[]> {
    return this.db
      .select()
      .from(tasks)
      .where(eq(tasks.status, status))
  }

  async findByUserId(userId: string): Promise<Task[]> {
    return this.db
      .select()
      .from(tasks)
      .where(eq(tasks.userId, userId))
  }

  async search(query: string): Promise<Task[]> {
    return this.db
      .select()
      .from(tasks)
      .where(like(tasks.title, `%${query}%`))
  }
}
```

## Transaction Support

```tsx
async create(data: NewTask, tx?: Transaction): Promise<Task> {
  const dbInstance = tx || this.db
  const [task] = await dbInstance
    .insert(tasks)
    .values(data)
    .returning()
  return task
}
```

## Relationships

```tsx
async findWithSubtasks(id: string): Promise<TaskWithSubtasks | null> {
  const task = await this.db
    .select()
    .from(tasks)
    .where(eq(tasks.id, id))
    .limit(1)

  if (!task[0]) return null

  const subtasks = await this.db
    .select()
    .from(tasks)
    .where(eq(tasks.parentId, id))

  return {
    ...task[0],
    subtasks
  }
}
```

## Access Control & Visibility

### Overview
Repositories must enforce access control for user-facing operations. The project uses visibility-based access control with two main patterns:

- **`withVisibilityScope()`**: For entities with `visibility` and `createdBy` fields (repositories, tasks)
- **`withUserScope()`**: For simple ownership-based access (messages, events)

### Read Access Control with Visibility

```tsx
import { withVisibilityScope } from '../../../infrastructure/database/database'

@injectable()
export class TaskRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Get a task by ID with visibility-based access control.
   * Returns null if task not found or user doesn't have access.
   * Applies visibility filtering: public tasks OR private tasks owned by user.
   */
  async get(taskId: string): Promise<Task | null> {
    const taskRows = await this.db.drizzle
      .select()
      .from(tasks)
      .where(
        withVisibilityScope(tasks, {
          additionalConditions: [eq(tasks.taskId, taskId)],
        }),
      )
      .limit(1)

    const taskRow = taskRows[0]
    if (!taskRow) {
      return null
    }

    return selectToTask(taskRow)
  }

  /**
   * List tasks with filtering, sorting, and pagination.
   * Applies visibility-based access control.
   */
  async listTasks(options: {
    limit?: number
    offset?: number
    status?: string
    repositoryId?: string
  }): Promise<{ tasks: Task[]; total: number }> {
    const { limit = 50, offset = 0, status, repositoryId } = options

    // Build WHERE conditions
    const conditions: SQL[] = []
    if (status) {
      conditions.push(eq(tasks.status, status))
    }
    if (repositoryId) {
      conditions.push(eq(tasks.repositoryId, repositoryId))
    }

    // Apply visibility-scoped access control
    const whereClause = withVisibilityScope(tasks, {
      additionalConditions: conditions.length > 0 ? conditions : [],
    })

    // Execute query with access control
    const taskRows = await this.db.drizzle
      .select()
      .from(tasks)
      .where(whereClause)
      .orderBy(desc(tasks.lastMessageAt))
      .limit(limit)
      .offset(offset)

    return { tasks: taskRows.map(selectToTask), total: taskRows.length }
  }
}
```

### Internal Methods (Bypass Access Control)

```tsx
/**
 * Get a task by ID without access control.
 * Used for internal/system operations like MCP tool execution.
 * INTERNAL USE ONLY - bypasses user access control.
 *
 * @param taskId - Task ID to fetch
 * @returns Task or null if not found
 */
async getInternal(taskId: string): Promise<Task | null> {
  const taskRows = await this.db.drizzle
    .select()
    .from(tasks)
    .where(eq(tasks.taskId, taskId))
    .limit(1)

  const taskRow = taskRows[0]
  if (!taskRow) {
    return null
  }

  return selectToTask(taskRow)
}

/**
 * List all tasks without access control.
 * Used for system-level operations like startup recovery and graceful shutdown.
 * INTERNAL USE ONLY - bypasses user access control.
 */
async listAll(): Promise<Task[]> {
  const taskRows = await this.db.drizzle
    .select()
    .from(tasks)
    .orderBy(desc(tasks.lastMessageAt))

  return taskRows.map(selectToTask)
}
```

### Write Permission Checks

```tsx
import { assertVisibilityWritePermission } from '../../../infrastructure/database/database'
import { assertOwnership } from '../../../features/auth'

/**
 * Delete a task by ID.
 * Verifies user ownership before deletion.
 * @throws {ForbiddenError} If user doesn't own the task
 */
async delete(taskId: string): Promise<void> {
  // Get task first to verify ownership
  const task = await this.get(taskId)
  if (!task) {
    // Task doesn't exist or user doesn't have access
    return
  }

  // Verify ownership (throws ForbiddenError if not authorized)
  assertOwnership(task)

  await this.db.drizzle.delete(tasks).where(eq(tasks.taskId, taskId))
  logger.debug({ taskId }, 'Task deleted from database')
}

/**
 * Update a repository.
 * Verifies user has write permission before update.
 * @throws {ForbiddenError} If user doesn't have write permission
 */
async update(id: string, data: Partial<Repository>): Promise<void> {
  // Get repository first to verify write permission
  const repository = await this.get(id)
  if (!repository) {
    throw new Error('Repository not found')
  }

  // Verify write permission (throws ForbiddenError if not authorized)
  assertVisibilityWritePermission(repository, 'repository')

  await this.db.drizzle
    .update(repositories)
    .set({ ...data, updatedAt: new Date() })
    .where(eq(repositories.id, id))
}
```

### Access Control Behavior

**`withVisibilityScope()`** returns records where:
- `visibility = 'public'` (all users can see)
- `visibility = 'private' AND createdBy = currentUserId` (owner can see)
- `createdBy IS NULL` (legacy records, visible to all)

**`withUserScope()`** returns records where:
- `createdBy = currentUserId` (owner can see)
- `createdBy IS NULL` (legacy records, visible to all)

**Dev Mode Behavior** (AUTH_PROVIDER=disabled):
- All access control is bypassed
- All records are returned regardless of ownership or visibility
- Used for local development without authentication

### Best Practices

✅ **DO: Apply access control to all user-facing queries**
```tsx
// Get with access control (user-facing)
async get(id: string): Promise<Task | null> {
  const rows = await this.db.drizzle
    .select()
    .from(tasks)
    .where(withVisibilityScope(tasks, {
      additionalConditions: [eq(tasks.id, id)]
    }))
  return rows[0] || null
}
```

✅ **DO: Clearly mark internal methods**
```tsx
/**
 * INTERNAL USE ONLY - bypasses user access control.
 */
async getInternal(id: string): Promise<Task | null> {
  // No access control
  const rows = await this.db.drizzle
    .select()
    .from(tasks)
    .where(eq(tasks.id, id))
  return rows[0] || null
}
```

✅ **DO: Check write permissions before updates/deletes**
```tsx
async delete(id: string): Promise<void> {
  const entity = await this.get(id)
  if (!entity) return

  // Verify user has write permission
  assertVisibilityWritePermission(entity, 'task')

  await this.db.drizzle.delete(tasks).where(eq(tasks.id, id))
}
```

✅ **DO: Document access control behavior**
```tsx
/**
 * Get a task by ID.
 * Returns null if task not found or user doesn't have access.
 * Applies visibility-based access control.
 */
async get(id: string): Promise<Task | null>
```

❌ **DON'T: Skip access control for user-facing operations**
```tsx
// WRONG - exposes all tasks regardless of ownership
async get(id: string): Promise<Task | null> {
  return this.db.drizzle.select().from(tasks).where(eq(tasks.id, id))
}
```

❌ **DON'T: Use internal methods for user-facing operations**
```tsx
// WRONG - bypasses access control
async getUserTask(taskId: string): Promise<Task | null> {
  return this.getInternal(taskId)  // Security vulnerability!
}
```

## Anti-Patterns

❌ **Business logic in repository**
```tsx
// WRONG
async create(data: NewTask): Promise<Task> {
  if (data.title.length < 3) {  // Business logic!
    throw new Error('Title too short')
  }
  return this.db.insert(tasks).values(data).returning()
}
```

❌ **Calling other services**
```tsx
// WRONG
@injectable()
class TaskRepository {
  constructor(
    @inject(TYPES.Database) private db: Database,
    @inject(TYPES.NotificationService) private notifications: INotificationService
  ) {}

  async create(data: NewTask): Promise<Task> {
    const task = await this.db.insert(tasks).values(data).returning()
    await this.notifications.send(task)  // Don't call services from repo!
    return task
  }
}
```
