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
