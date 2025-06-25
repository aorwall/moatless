import { randomUUID } from 'node:crypto'
import type { FastifyInstance } from 'fastify'
import { TaskNotFoundError, ValidationError } from '../../shared/constants/errors.js'
import { EventType } from '../../shared/types/event.js'
import type { CreateTaskRequest, Task, TaskIteration } from '../../shared/types/task.js'
import type { EnvironmentService } from './environment.service.js'
import type { EventService } from './event.service.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

export class TaskService {
  private backgroundTasks = new Set<Promise<void>>()

  constructor(
    private storage: StorageBackend,
    private eventService: EventService,
    private environmentService: EnvironmentService,
    private workspaceService: WorkspaceService,
  ) {}

  async createTask(params: CreateTaskRequest): Promise<string> {
    // Validate environment exists
    const environment = await this.environmentService.getEnvironment(params.environmentId)
    if (!environment) {
      throw new ValidationError(`Environment ${params.environmentId} not found`, {
        environmentId: params.environmentId,
      })
    }

    const taskId = randomUUID()
    const now = new Date()

    const metadata: Task = {
      taskId,
      createdAt: now,
      lastMessageAt: now,
      agentType: params.agentType,
      environmentId: params.environmentId,
      status: 'active',
      numTurns: 0,
      totalCostUsd: 0.0,
      iterations: [],
    }

    await this.saveMetadata(taskId, metadata)

    // Emit task creation event
    await this.eventService.emitEvent({
      eventId: randomUUID(),
      type: EventType.START,
      taskId,
      timestamp: now,
      isError: false,
      payload: {
        agentType: params.agentType,
        environmentId: params.environmentId,
      },
    })

    // Setup workspace asynchronously in background
    const setupTask = this.setupWorkspaceAsync(
      taskId,
      metadata,
      params.branchName,
      params.message,
      params.start ?? true,
    )

    this.backgroundTasks.add(setupTask)
    setupTask.finally(() => this.backgroundTasks.delete(setupTask))

    return taskId
  }

  async getTask(taskId: string): Promise<Task | null> {
    return this.storage.getTask(taskId)
  }

  async listTasks(limit = 50, offset = 0): Promise<Task[]> {
    const taskKeys = await this.storage.list('tasks/')
    const tasks: Task[] = []

    for (const key of taskKeys) {
      const taskId = key.replace('tasks/', '')
      const task = await this.storage.getTask(taskId)
      if (task) tasks.push(task)
    }

    // Sort by last message time (most recent first)
    tasks.sort((a, b) => b.lastMessageAt.getTime() - a.lastMessageAt.getTime())

    // Apply pagination
    return tasks.slice(offset, offset + limit)
  }

  async sendMessage(taskId: string, message: string): Promise<void> {
    const metadata = await this.getTask(taskId)
    if (!metadata) {
      throw new TaskNotFoundError(taskId)
    }

    // Update metadata
    const now = new Date()
    const iterationNumber = metadata.iterations.length + 1

    const iteration: TaskIteration = {
      iterationNumber,
      startTime: now,
      status: 'active',
      messageCount: 1,
      costUsd: 0,
      initialMessage: message,
    }

    metadata.iterations.push(iteration)
    metadata.lastMessageAt = now
    metadata.numTurns += 1

    await this.saveMetadata(taskId, metadata)

    // Emit input event
    await this.eventService.emitEvent({
      eventId: randomUUID(),
      type: EventType.INPUT,
      taskId,
      iterationId: iterationNumber,
      timestamp: now,
      isError: false,
      payload: { message },
    })

    // Get or create workspace
    const workspace = await this.workspaceService.getWorkspace(taskId)
    if (!workspace) {
      throw new ValidationError(`Workspace not found for task ${taskId}`, { taskId })
    }

    // Check workspace status and setup if needed
    const status = workspace.getStatus()
    if (status === 'running') {
      throw new ValidationError(`Task ${taskId} is already running`, { taskId })
    } else if (status === 'setting_up') {
      // Wait for setup to complete
      console.log(`Waiting for workspace setup to complete for task ${taskId}`)
      // TODO: Implement wait logic
    }

    // If workspace is not setup, run setup first
    if (status === 'not_setup') {
      await workspace.setup(iterationNumber)
    }

    // Trigger the agent
    await workspace.triggerAgent(metadata.agentType, message, iterationNumber)
  }

  async updateTask(taskId: string, updates: Partial<Task>): Promise<void> {
    const metadata = await this.getTask(taskId)
    if (!metadata) {
      throw new TaskNotFoundError(taskId)
    }

    const updatedMetadata = { ...metadata, ...updates }
    await this.saveMetadata(taskId, updatedMetadata)
  }

  async deleteTask(taskId: string): Promise<void> {
    const task = await this.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Clean up workspace
    await this.workspaceService.cleanupWorkspace(taskId)

    // Delete task data
    await this.storage.delete(`tasks/${taskId}`)
    await this.storage.delete(`events/${taskId}`)

    // Emit deletion event
    await this.eventService.emitEvent({
      eventId: randomUUID(),
      type: EventType.STOP,
      taskId,
      timestamp: new Date(),
      isError: false,
      payload: { reason: 'deleted' },
    })
  }

  private async saveMetadata(_taskId: string, metadata: Task): Promise<void> {
    await this.storage.saveTask(metadata)
  }

  private async setupWorkspaceAsync(
    taskId: string,
    metadata: Task,
    branchName?: string,
    message?: string,
    start?: boolean,
  ): Promise<void> {
    try {
      // Create workspace
      await this.workspaceService.createWorkspace(taskId, metadata.environmentId, branchName)

      // Emit setup complete event
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: EventType.WORKSPACE,
        subType: 'setup_completed',
        taskId,
        timestamp: new Date(),
        isError: false,
        payload: { workspaceId: taskId },
      })

      if (start && message) {
        await this.sendMessage(taskId, message)
      }
    } catch (error) {
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: EventType.ERROR,
        taskId,
        timestamp: new Date(),
        isError: true,
        payload: {
          error: error instanceof Error ? error.message : String(error),
          phase: 'workspace_setup',
        },
      })
    }
  }

  async waitForBackgroundTasks(): Promise<void> {
    await Promise.all(Array.from(this.backgroundTasks))
  }
}

export function createTaskService(
  _app: FastifyInstance,
  storage: StorageBackend,
  eventService: EventService,
  environmentService: EnvironmentService,
  workspaceService: WorkspaceService,
): TaskService {
  return new TaskService(storage, eventService, environmentService, workspaceService)
}
