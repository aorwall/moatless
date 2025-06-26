import type { FastifyInstance } from 'fastify'
import { randomUUID } from 'node:crypto'
import {
  CriticalError,
  TaskNotFoundError,
} from '../../shared/constants/errors.js'
import type { CreateTaskRequest, Task, TaskIteration } from '../../shared/types/task.js'
import { createTaskLogger, logger } from '../utils/logger.js'
import type { EventService } from './event.service.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

export class TaskService {
  constructor(
    private storage: StorageBackend,
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

    try {
      const workspace = await this.workspaceService.createWorkspace(
        task.taskId,
        params.environmentId,
        params.branchName,
      )
      task.workspacePath = workspace.getWorkspacePath()
    } catch (error) {
      logger.error('Failed to create workspace', error as Error, {
        taskId: task.taskId,
        environmentId: params.environmentId,
      })
      throw error
    }

    try {
      await this.storage.saveTask(task)

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

    // Track background task in WorkspaceService
    this.workspaceService.trackBackgroundTask(task.taskId, setupTask)

    logger.info('Task creation completed successfully', { taskId: task.taskId })
    return task.taskId
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

    await this.storage.saveTask(metadata)

    // Pass the full iteration object with task reference
    iteration.task = metadata
    iteration.taskId = taskId

    await this.workspaceService.runAgent(taskId, message, iteration)

  }

  async updateTask(taskId: string, updates: Partial<Task>): Promise<void> {
    const metadata = await this.getTask(taskId)
    if (!metadata) {
      throw new TaskNotFoundError(taskId)
    }

    const updatedMetadata = { ...metadata, ...updates }
    await this.storage.saveTask(updatedMetadata)
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

        // Wait for workspace to be ready (max 30 seconds)
        const maxWaitTime = 30000
        const checkInterval = 500
        const startWait = Date.now()

        while (Date.now() - startWait < maxWaitTime) {
          const workspace = await this.workspaceService.getWorkspace(taskId)
          if (workspace && workspace.getStatus() === 'idle') {
            break
          }
          await new Promise((resolve) => setTimeout(resolve, checkInterval))
        }

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

      // Create a failed iteration to capture the setup error
      const iteration: TaskIteration = {
        iterationNumber: 1,
        startTime: new Date(startTime),
        endTime: new Date(),
        status: 'failed',
        messageCount: 0,
        costUsd: 0,
        initialMessage: message,
        durationMs: setupDuration,
        error: {
          message: error instanceof Error ? error.message : String(error),
          stack: error instanceof Error ? error.stack : undefined,
          timestamp: new Date(),
        },
      }

      // Update task metadata with the failed iteration
      metadata.iterations.push(iteration)
      metadata.status = 'error'
      await this.storage.saveTask(metadata)

      // Re-throw if this is a critical error
      if (error instanceof CriticalError) {
        throw error
      }
    }
  }
}

export function createTaskService(
  _app: FastifyInstance,
  storage: StorageBackend,
  _eventService: EventService,
  workspaceService: WorkspaceService,
): TaskService {
  return new TaskService(storage, workspaceService)
}
