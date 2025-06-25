import { randomUUID } from 'node:crypto'
import type { FastifyInstance } from 'fastify'
import { TaskNotFoundError, ValidationError } from '../../shared/constants/errors.js'
import { EventType } from '../../shared/types/event.js'
import type { CreateTaskRequest, Task, TaskIteration } from '../../shared/types/task.js'
import {createTaskLogger, logger} from '../utils/logger.js'
import type { EventService } from './event.service.js'
import type { TaskStorageService } from './task-storage.service.js'
import type { WorkspaceService } from './workspace.service.js'
import {BaseAgent} from "@server/services/agents/base-agent.js";

export class TaskService {
  private backgroundTasks = new Set<Promise<void>>()

  constructor(
    private taskStorageService: TaskStorageService,
    private eventService: EventService,
    private workspaceService: WorkspaceService,
  ) {}

  async createTask(params: CreateTaskRequest): Promise<string> {
    logger.info('Starting task creation', {
      agentType: params.agentType,
      environmentId: params.environmentId,
      hasBranchName: !!params.branchName,
      hasMessage: !!params.message,
      start: params.start ?? true,
    })

    const now = new Date()
    const task: Task = {
      taskId: randomUUID(),
      createdAt: now,
      lastMessageAt: now,
      agentType: params.agentType,
      environmentId: params.environmentId,
      status: 'active',
      numTurns: 0,
      totalCostUsd: 0.0,
      iterations: [],
    }

    const workspace = await this.workspaceService.createWorkspace(task.taskId, params.environmentId)
    task.workspacePath = workspace.getWorkspacePath()

    try {
      await this.saveMetadata(task.taskId, task)

      logger.info('Task metadata created successfully', {
        taskId: task.taskId,
        agentType: params.agentType,
        environmentId: params.environmentId,
        workspacePath: task.workspacePath,
      })
    } catch (error) {
      logger.error('Failed to save task metadata', error as Error)
      throw error
    }

    const setupTask = this.setupWorkspaceAsync(
      task.taskId,
      task,
      params.branchName,
      params.message,
      params.start ?? true,
    )

    this.backgroundTasks.add(setupTask)

    setupTask.finally(() => {
      this.backgroundTasks.delete(setupTask)
      logger.debug('Background workspace setup task completed')
    })

    logger.info('Task creation completed successfully', { taskId: task.taskId })
    return task.taskId
  }

  async getTask(taskId: string): Promise<Task | null> {
    return this.taskStorageService.getTask(taskId)
  }

  async listTasks(limit = 50, offset = 0): Promise<Task[]> {
    const taskKeys = await this.taskStorageService.listTasks()
    const tasks: Task[] = []

    for (const key of taskKeys) {
      const taskId = key.replace('tasks/', '')
      const task = await this.taskStorageService.getTask(taskId)
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

    // Get or create workspace
    const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)

    // Check workspace status and setup if needed
    const taskLogger = createTaskLogger(taskId)
    const status = workspace.getStatus()

    taskLogger.debug('Workspace status check', { status })

    if (status === 'running') {
      taskLogger.warn('Attempted to send message to running task', { status })
      throw new ValidationError(`Task ${taskId} is already running`, { taskId })
    } else if (status === 'setting_up') {
      // Wait for setup to complete
      taskLogger.info('Waiting for workspace setup to complete')
      // TODO: Implement wait logic
    }

    // Start workspace (will run setup if needed and trigger agent)
    const agent: BaseAgent = await this.workspaceService.startWorkspace(taskId, metadata.agentType, message, iterationNumber)
    iteration.agentSessionId = agent.getSessionId()
    await this.saveMetadata(taskId, metadata)
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
    await this.taskStorageService.deleteTask(taskId)
    await this.taskStorageService.deleteTaskEvents(taskId)

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
    await this.taskStorageService.saveTask(metadata)
  }

  private async setupWorkspaceAsync(
    taskId: string,
    metadata: Task,
    branchName?: string,
    message?: string,
    start?: boolean,
  ): Promise<void> {
    const taskLogger = createTaskLogger(taskId)
    const startTime = Date.now()

    taskLogger.info('Starting workspace setup', {
      environmentId: metadata.environmentId,
      hasBranchName: !!branchName,
      hasMessage: !!message,
      willStart: start ?? true,
    })

    try {
      // Setup workspace in background
      await this.workspaceService.setupWorkspace(taskId)

      const setupDuration = Date.now() - startTime
      taskLogger.info('Workspace created successfully', {
        durationMs: setupDuration,
        environmentId: metadata.environmentId,
      })

      taskLogger.debug('Workspace setup complete event emitted')

      if (start && message) {
        taskLogger.info('Starting initial message processing')
        await this.sendMessage(taskId, message)
      }

      taskLogger.info('Workspace setup completed successfully', {
        totalDurationMs: Date.now() - startTime,
      })
    } catch (error) {
      const setupDuration = Date.now() - startTime
      taskLogger.error('Workspace setup failed', error as Error, {
        durationMs: setupDuration,
        environmentId: metadata.environmentId,
        phase: 'workspace_setup',
      })

      throw error
    }
  }

  async waitForBackgroundTasks(): Promise<void> {
    await Promise.all(Array.from(this.backgroundTasks))
  }
}

export function createTaskService(
  _app: FastifyInstance,
  taskStorageService: TaskStorageService,
  eventService: EventService,
  workspaceService: WorkspaceService,
): TaskService {
  return new TaskService(taskStorageService, eventService, workspaceService)
}
