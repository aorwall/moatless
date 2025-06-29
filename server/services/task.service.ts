import { randomUUID } from 'node:crypto'
import path from 'node:path'
import type { FastifyInstance } from 'fastify'
import { TaskNotFoundError } from '../../shared/constants/errors.js'
import type { CreateTaskRequest, Task, TaskIteration } from '../../shared/types/task.js'
import { logger } from '../utils/logger.js'
import type { AgentService } from './agent.service.js'
import type { CommandService } from './command.service.js'
import type { EventService } from './event.service.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

export class TaskService {
  private static readonly WORKSPACE_READY_TIMEOUT = 30000
  private static readonly WORKSPACE_CHECK_INTERVAL = 500
  private static readonly FILE_MIME_TYPES: Record<string, string> = {
    '.jpg': 'image/jpeg',
    '.jpeg': 'image/jpeg',
    '.png': 'image/png',
    '.gif': 'image/gif',
    '.webp': 'image/webp',
    '.pdf': 'application/pdf',
    '.txt': 'text/plain',
    '.md': 'text/markdown',
    '.json': 'application/json',
  }

  constructor(
    private storage: StorageBackend,
    private workspaceService: WorkspaceService,
    private agentService: AgentService,
    private commandService: CommandService,
  ) {}

  async createTask(params: CreateTaskRequest): Promise<string> {
    logger.info('Starting task creation', {
      agentType: params.agentType,
      environmentId: params.environmentId,
      hasBranchName: !!params.branchName,
      hasMessage: !!params.message,
      hasFilePaths: !!(params.filePaths && params.filePaths.length > 0),
      start: params.start ?? true,
    })

    const now = new Date()
    const task: Task = {
      taskId: randomUUID(),
      createdAt: now,
      lastMessageAt: now,
      agentType: params.agentType,
      environmentId: params.environmentId,
      status: 'open',
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

    // Determine if we should start immediately
    const shouldStart = (params.start ?? true) && !!params.message

    // Setup workspace
    await this.workspaceService.setupWorkspace(task.taskId)

    // If we should start immediately, do it synchronously
    if (shouldStart) {
      // Wait for workspace to be ready
      const maxWaitTime = TaskService.WORKSPACE_READY_TIMEOUT
      const checkInterval = TaskService.WORKSPACE_CHECK_INTERVAL
      const startWait = Date.now()

      while (Date.now() - startWait < maxWaitTime) {
        const workspace = await this.workspaceService.getWorkspace(task.taskId)
        if (workspace && workspace.getStatus() === 'idle') {
          break
        }
        await new Promise((resolve) => setTimeout(resolve, checkInterval))
      }

      // Update task status to running before sending message
      task.status = 'running'
      await this.storage.saveTask(task)

      // Send the initial message
      const sendMessageTask = this.sendMessage(
        task.taskId,
        params.message!,
        true,
        params.model,
        undefined,
        params.filePaths,
      ).catch((error) => {
        logger.error('Failed to send initial message', error as Error, { taskId: task.taskId })
        // Update status back to open on error
        task.status = 'open'
        this.storage.saveTask(task).catch((saveError) => {
          logger.error('Failed to update task status after error', saveError as Error)
        })
      })

      // Track the background message sending task
      this.workspaceService.trackBackgroundTask(task.taskId, sendMessageTask)
    }

    logger.info('Task creation completed successfully', { taskId: task.taskId })
    return task.taskId
  }

  async getTask(taskId: string): Promise<Task> {
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // If task exists and status is running, check if agent is still running
    if (task && task.status === 'running') {
      const isAgentRunning = await this.agentService.getTaskRunningStatus(taskId)

      // If agent is not running, update task status to open
      if (!isAgentRunning) {
        task.status = 'open'
        await this.storage.saveTask(task)
      }
    }

    return task
  }

  async listTasks(limit = 50, offset = 0, status?: string): Promise<Task[]> {
    const taskKeys = await this.storage.list('tasks/')
    const tasks: Task[] = []

    for (const key of taskKeys) {
      const taskId = key.replace('tasks/', '')
      const task = await this.storage.getTask(taskId)
      if (task) {
        // Filter by status if provided
        if (!status || task.status === status) {
          tasks.push(task)
        }
      }
    }

    // Sort by last message time (most recent first)
    tasks.sort((a, b) => b.lastMessageAt.getTime() - a.lastMessageAt.getTime())

    // Apply pagination
    return tasks.slice(offset, offset + limit)
  }

  async sendMessage(
    taskId: string,
    message: string,
    resume = true,
    model?: string,
    fileIds?: string[],
    filePaths?: string[],
    commandName?: string,
  ): Promise<void> {
    const metadata = await this.getTask(taskId)

    // If commandName is provided, get the command prompt
    let finalMessage = message
    if (commandName) {
      const command = await this.commandService.getCommand(commandName)
      if (command) {
        finalMessage = command.prompt
      } else {
        logger.warn('Command not found, using original message', { commandName })
      }
    }

    // Update metadata
    const now = new Date()
    const iterationNumber = metadata.iterations.length + 1

    // Find the previous iteration's session ID for resume
    let resumeSessionId: string | undefined
    if (resume && metadata.iterations.length > 0) {
      const previousIteration = metadata.iterations[metadata.iterations.length - 1]
      resumeSessionId = previousIteration.agentSessionId
    }

    const iteration: TaskIteration = {
      iterationNumber,
      startTime: now,
      status: 'active',
      messageCount: 1,
      costUsd: 0,
      initialMessage: finalMessage,
      commandName,
    }

    // Add file attachments if provided
    if (filePaths && filePaths.length > 0) {
      iteration.attachments = []
      for (const filePath of filePaths) {
        // Extract filename from path
        const filename = path.basename(filePath)
        // Try to determine mime type from extension
        const ext = path.extname(filename).toLowerCase()
        const mimeType = TaskService.FILE_MIME_TYPES[ext] || 'application/octet-stream'

        iteration.attachments.push({
          filename,
          path: filePath,
          mimeType,
        })
      }
    } else if (fileIds && fileIds.length > 0) {
      // Backward compatibility with fileIds
      iteration.attachments = []
      for (const fileId of fileIds) {
        // Get file metadata
        const fileMetadata = await this.storage.get<any>(`uploads/${taskId}/${fileId}`)
        if (fileMetadata) {
          iteration.attachments.push({
            fileId,
            filename: fileMetadata.originalFilename,
            path: fileMetadata.path,
            mimeType: fileMetadata.mimeType,
          })
        }
      }
    }

    metadata.iterations.push(iteration)
    metadata.lastMessageAt = now
    metadata.numTurns += 1
    metadata.status = 'running'

    await this.storage.saveTask(metadata)

    // Pass the full iteration object with task reference
    iteration.task = metadata
    iteration.taskId = taskId

    // Execute agent through AgentService in background
    const backgroundTask = this.agentService
      .executeAgent(taskId, finalMessage, iteration, 'claude-code', resumeSessionId, model)
      .catch((error) => {
        logger.error('Background agent execution failed', error as Error, { taskId })
      })

    // Track the background task in WorkspaceService for status reporting
    this.workspaceService.trackBackgroundTask(taskId, backgroundTask)
  }

  async updateTask(taskId: string, updates: Partial<Task>): Promise<void> {
    const metadata = await this.getTask(taskId)

    const updatedMetadata = { ...metadata, ...updates }
    await this.storage.saveTask(updatedMetadata)
  }

  async deleteTask(taskId: string): Promise<void> {
    await this.getTask(taskId)

    // Clean up workspace
    await this.workspaceService.cleanupWorkspace(taskId)

    // Delete task data
    await this.storage.delete(`tasks/${taskId}`)
    await this.storage.delete(`events/${taskId}`)
  }

  async closeTask(taskId: string): Promise<void> {
    const task = await this.getTask(taskId)

    logger.debug('Closing task', { taskId })

    // Stop any running agent
    await this.agentService.stopAgent(taskId)

    // Stop preview server if running
    try {
      await this.workspaceService.stopServer(taskId)
    } catch (error) {
      // Log error but continue with cleanup
      logger.warn('Failed to stop server during task close', error as Error)
    }

    // Clean up workspace (this will teardown the workspace)
    await this.workspaceService.cleanupWorkspace(taskId)

    // Update task status to closed
    task.status = 'closed'
    await this.storage.saveTask(task)

    logger.info('Task closed successfully', { taskId })
  }
}

export function createTaskService(
  _app: FastifyInstance,
  storage: StorageBackend,
  _eventService: EventService,
  workspaceService: WorkspaceService,
  agentService: AgentService,
  commandService: CommandService,
): TaskService {
  return new TaskService(storage, workspaceService, agentService, commandService)
}
