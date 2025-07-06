import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { TaskNotFoundError, ValidationError } from '@moatless/shared/constants/errors'
import type { CreateTaskRequest, Task, TaskIteration } from '@moatless/shared/types/task'
import type { CreateVibeTaskRequest } from '@moatless/shared/types/api'
import { RepositoryTemplate } from '@moatless/shared/types/repository'
import { EventType } from '@moatless/shared/types/event'
import { logger } from '../utils/logger.js'
import type { AgentService } from './agent.service.js'
import type { CommandService } from './command.service.js'
import type { EventService } from './event.service.js'
import type { RepositoryService } from './repository.service.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

interface FileMetadata {
  originalFilename: string
  path: string
  mimeType: string
}

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
    private eventService: EventService,
    private workspaceService: WorkspaceService,
    private agentService: AgentService,
    private commandService: CommandService,
    private repositoryService: RepositoryService,
  ) {}

  async createTask(params: CreateTaskRequest): Promise<string> {
    logger.info('Starting task creation', {
      agentType: params.agentType,
      repositoryId: params.repositoryId,
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
      repositoryId: params.repositoryId,
      status: 'open',
      numTurns: 0,
      totalCostUsd: 0.0,
      iterations: [],
    }

    try {
      const workspace = await this.workspaceService.createWorkspace(
        task.taskId,
        params.repositoryId,
        params.originalBranch || params.branchName, // Use originalBranch if provided, fallback to branchName for backward compatibility
      )
      task.workspacePath = workspace.getWorkspacePath()
    } catch (error) {
      logger.error('Failed to create workspace', error as Error, {
        taskId: task.taskId,
        repositoryId: params.repositoryId,
      })
      throw error
    }

    try {
      await this.storage.saveTask(task)

      logger.info('Task metadata created successfully', {
        taskId: task.taskId,
        agentType: params.agentType,
        repositoryId: params.repositoryId,
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

  async createVibeTask(params: CreateVibeTaskRequest): Promise<string> {
    logger.info('Starting vibe task creation from template', {
      agentType: params.agentType,
      templateId: params.templateId,
      repositoryName: params.repositoryName,
      hasMessage: !!params.message,
      hasFilePaths: !!(params.filePaths && params.filePaths.length > 0),
      start: params.start ?? true,
    })

    // Define available templates - in future this could be stored in database
    const templates = [
      new RepositoryTemplate(
        'hello-world-ui',
        'Basic Hello Website',
        'https://github.com/aorwall/hello-world-ui.git',
        'npm install',
        {
          serverStartCommand: 'npm run dev',
          serverPort: 3000,
          injectionStrategy: {
            method: 'html-injection',
          },
        }
      ),
    ]

    // Find the template
    const template = templates.find(t => t.id === params.templateId)
    if (!template) {
      throw new ValidationError(`Template with id ${params.templateId} not found`, 'templateId')
    }

    // Create a directory for cloning the template
    const dataDir = process.env.DATA_DIR || './data'
    const templatesDir = path.join(dataDir, 'templates')
    const templateRepoDir = path.join(templatesDir, template.id)
    
    // Ensure templates directory exists
    await fs.mkdir(templatesDir, { recursive: true })
    
    // Clone or update the template repository if it doesn't exist
    if (!(await this.pathExists(templateRepoDir))) {
      logger.info('Cloning template repository', {
        templateId: template.id,
        gitUrl: template.gitUrl,
        targetDir: templateRepoDir,
      })
      
      const { exec } = await import('node:child_process')
      const { promisify } = await import('node:util')
      const execAsync = promisify(exec)
      
      await execAsync(`git clone ${template.gitUrl} ${template.id}`, {
        cwd: templatesDir,
        timeout: 30000, // 30 seconds for clone
      })
    }
    
    // Create a new repository from the template
    const repository = await this.repositoryService.createRepository({
      name: params.repositoryName,
      description: `Created from template: ${template.name}`,
      path: templateRepoDir,
      useGitWorktrees: true,
      codeExecution: {
        type: 'local',
        setupCommands: template.setupCommand,
      },
      previewConfig: template.previewConfig,
    })

    // Create a task with the new repository
    const createTaskParams: CreateTaskRequest = {
      agentType: params.agentType,
      repositoryId: repository.id,
      message: params.message,
      start: params.start,
      model: params.model,
      filePaths: params.filePaths,
    }

    return this.createTask(createTaskParams)
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
      initialMessage: message,
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
        const fileMetadata = await this.storage.get<FileMetadata>(`uploads/${taskId}/${fileId}`)
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
    const previousStatus = metadata.status
    metadata.status = 'running'

    await this.storage.saveTask(metadata)
    
    // Emit status change event
    await this.eventService.emitEvent({
      taskId,
      type: EventType.INPUT,
      subType: 'task:statusChanged',
      payload: {
        status: 'running',
        previousStatus,
        iterationNumber
      },
      eventId: randomUUID(),
      timestamp: now,
      agentType: metadata.agentType,
      isError: false
    })

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
      logger.warn('Failed to stop server during task close', {
        error: error instanceof Error ? error.message : String(error),
      })
    }

    // Clean up workspace (this will teardown the workspace)
    await this.workspaceService.cleanupWorkspace(taskId)

    // Update task status to closed
    const previousStatus = task.status
    task.status = 'closed'
    await this.storage.saveTask(task)
    
    // Emit status change event
    await this.eventService.emitEvent({
      taskId,
      type: EventType.INPUT,
      subType: 'task:statusChanged',
      payload: {
        status: 'closed',
        previousStatus
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      agentType: task.agentType,
      isError: false
    })

    logger.info('Task closed successfully', { taskId })
  }

  private async pathExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }
}

export function createTaskService(
  storage: StorageBackend,
  eventService: EventService,
  workspaceService: WorkspaceService,
  agentService: AgentService,
  commandService: CommandService,
  repositoryService: RepositoryService,
): TaskService {
  return new TaskService(storage, eventService, workspaceService, agentService, commandService, repositoryService)
}
