# Service Layer

## Overview
Services contain business logic and orchestrate repositories. They are independent of HTTP concerns.

## Basic Service Pattern

```tsx
import { injectable, inject } from 'inversify'
import { TYPES } from '../types'
import type { ITaskRepository } from '../repositories/task.repository.interface'

@injectable()
export class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: ITaskRepository
  ) {}

  async getAllTasks(): Promise<Task[]> {
    return this.taskRepo.findAll()
  }

  async getTaskById(id: string): Promise<Task | null> {
    return this.taskRepo.findById(id)
  }

  async createTask(data: CreateTaskData): Promise<Task> {
    // Business logic: validate data
    if (data.title.length < 3) {
      throw new Error('Title must be at least 3 characters')
    }

    // Business logic: set defaults
    const task = {
      ...data,
      status: 'pending' as const,
      createdAt: new Date(),
      updatedAt: new Date()
    }

    return this.taskRepo.create(task)
  }

  async updateTask(id: string, data: Partial<Task>): Promise<Task> {
    const existing = await this.taskRepo.findById(id)
    if (!existing) {
      throw new Error('Task not found')
    }

    // Business logic: prevent status changes for completed tasks
    if (existing.status === 'completed' && data.status) {
      throw new Error('Cannot change status of completed task')
    }

    return this.taskRepo.update(id, {
      ...data,
      updatedAt: new Date()
    })
  }

  async deleteTask(id: string): Promise<void> {
    const task = await this.taskRepo.findById(id)
    if (!task) {
      throw new Error('Task not found')
    }

    // Business logic: prevent deletion of running tasks
    if (task.status === 'running') {
      throw new Error('Cannot delete running task')
    }

    await this.taskRepo.delete(id)
  }
}
```

## Multi-Repository Orchestration

```tsx
@injectable()
export class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: ITaskRepository,
    @inject(TYPES.UserRepository) private userRepo: IUserRepository,
    @inject(TYPES.NotificationService) private notificationService: INotificationService
  ) {}

  async assignTask(taskId: string, userId: string): Promise<Task> {
    // Validate task exists
    const task = await this.taskRepo.findById(taskId)
    if (!task) {
      throw new Error('Task not found')
    }

    // Validate user exists
    const user = await this.userRepo.findById(userId)
    if (!user) {
      throw new Error('User not found')
    }

    // Business logic: check user capacity
    const userTasks = await this.taskRepo.findByUserId(userId)
    if (userTasks.filter(t => t.status === 'running').length >= 3) {
      throw new Error('User has too many active tasks')
    }

    // Update task
    const updatedTask = await this.taskRepo.update(taskId, {
      userId,
      assignedAt: new Date()
    })

    // Send notification
    await this.notificationService.sendTaskAssigned(updatedTask, user)

    return updatedTask
  }
}
```

## Transaction Management

```tsx
@injectable()
export class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: ITaskRepository,
    @inject(TYPES.Database) private db: Database
  ) {}

  async createTaskWithSubtasks(
    taskData: CreateTaskData,
    subtasks: CreateSubtaskData[]
  ): Promise<Task> {
    return this.db.transaction(async (tx) => {
      // Create main task
      const task = await this.taskRepo.create(taskData, tx)

      // Create subtasks
      for (const subtaskData of subtasks) {
        await this.taskRepo.createSubtask({
          ...subtaskData,
          parentId: task.id
        }, tx)
      }

      return task
    })
  }
}
```

## Authorization Logic

```tsx
@injectable()
export class TaskService {
  async updateTask(id: string, data: Partial<Task>, userId: string): Promise<Task> {
    const task = await this.taskRepo.findById(id)
    if (!task) {
      throw new Error('Task not found')
    }

    // Business logic: authorization check
    if (task.userId !== userId) {
      throw new Error('Not authorized to update this task')
    }

    return this.taskRepo.update(id, data)
  }
}
```

## Anti-Patterns

❌ **HTTP concerns in service**
```tsx
// WRONG
class TaskService {
  async getTasks(req: Request): Promise<Response> {  // HTTP objects!
    const tasks = await this.taskRepo.findAll()
    return new Response(JSON.stringify(tasks))
  }
}

// CORRECT
class TaskService {
  async getTasks(): Promise<Task[]> {
    return this.taskRepo.findAll()
  }
}
```

❌ **Database access without repository**
```tsx
// WRONG
@injectable()
class TaskService {
  constructor(@inject(TYPES.Database) private db: Database) {}

  async getTasks(): Promise<Task[]> {
    return this.db.select().from(tasks)  // Direct DB access!
  }
}

// CORRECT
@injectable()
class TaskService {
  constructor(@inject(TYPES.TaskRepository) private taskRepo: ITaskRepository) {}

  async getTasks(): Promise<Task[]> {
    return this.taskRepo.findAll()
  }
}
```

## Related Patterns
- [Layered Architecture](./layered-architecture.md) - Service layer role
- [Dependency Injection](./dependency-injection.md) - Service setup
- [Repository Pattern](./repository-pattern.md) - Data access
