import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import type { CreateVibeTaskRequest } from '@moatless/types/types/api'
import type { Command } from '@moatless/types/types/command'
import { EventType } from '@moatless/types/types/event'
import { RepositoryTemplate } from '@moatless/types/types/repository'
import type { CreateTaskRequest, Task, TaskIteration } from '@moatless/types/types/task'
import { filterAvailableCommands } from '../utils/command-conditions.js'
import { getDataDir } from '../utils/data-dir.js'
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

  async createTask(params: CreateTaskRequest): Promise<Task> {
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

    // Create task iteration with 'created' status if message is provided
    if (params.message) {
      const iteration = await this.createTaskIteration(
        task,
        params.message,
        undefined,
        params.model,
        undefined,
        params.filePaths,
        'created',
      )
      task.iterations.push(iteration)
    }

    try {
      await this.storage.saveTask(task)

      // Emit task created event
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: EventType.TASK_STATUS,
        subType: 'open',
        payload: {
          status: 'open',
          previousStatus: 'created',
        },
        eventId: randomUUID(),
        timestamp: now,
        agentType: params.agentType,
        isError: false,
      })

      logger.info('Task metadata created successfully', {
        taskId: task.taskId,
        agentType: params.agentType,
        repositoryId: params.repositoryId,
        hasInitialIteration: !!params.message,
      })
    } catch (error) {
      logger.error('Failed to save task metadata', error as Error)
      throw error
    }

    try {
      const workspace = await this.workspaceService.createWorkspace(
        task.taskId,
        params.repositoryId,
        params.originalBranch || params.branchName, // Use originalBranch if provided, fallback to branchName for backward compatibility
      )
      task.workspacePath = workspace.getWorkspacePath()

      // Save task with workspace path
      await this.storage.saveTask(task)
    } catch (error) {
      logger.error('Failed to create workspace', error as Error, {
        taskId: task.taskId,
        repositoryId: params.repositoryId,
      })
      throw error
    }

    // Return task immediately - workspace setup will run in background
    logger.info('Task creation completed successfully', { taskId: task.taskId })

    // Setup workspace and optionally start task in background
    this.setupWorkspaceInBackground(task.taskId, params.start, params.message)

    return task
  }

  /**
   * Sets up workspace and optionally starts task in background
   */
  private async setupWorkspaceInBackground(
    taskId: string,
    start?: boolean,
    message?: string,
  ): Promise<void> {
    try {
      logger.info('Starting background workspace setup', { taskId })

      // Setup workspace
      await this.workspaceService.setupWorkspace(taskId)

      // Determine if we should start immediately
      const shouldStart = (start ?? true) && !!message

      // If we should start immediately, start the task
      if (shouldStart) {
        try {
          await this.startTask(taskId)
        } catch (error) {
          logger.error('Failed to start task after workspace setup', error as Error, { taskId })
          // Don't throw here - workspace was set up successfully
        }
      }

      logger.info('Background workspace setup completed', { taskId, started: shouldStart })
    } catch (error) {
      logger.error('Background workspace setup failed', error as Error, { taskId })

      // Update task status to error
      try {
        const task = await this.storage.getTask(taskId)
        if (task) {
          const previousStatus = task.status
          task.status = 'error'
          await this.storage.saveTask(task)

          // Emit status change event
          await this.eventService.emitEvent({
            eventId: randomUUID(),
            taskId,
            type: EventType.TASK_STATUS,
            subType: 'error',
            isError: true,
            agentType: task.agentType,
            payload: {
              status: 'error',
              previousStatus,
              error: error instanceof Error ? error.message : String(error),
            },
          })
        }
      } catch (updateError) {
        logger.error(
          'Failed to update task status after workspace setup failure',
          updateError as Error,
          { taskId },
        )
      }
    }
  }

  /**
   * Starts a task by activating the last iteration with 'created' status
   */
  async startTask(taskId: string): Promise<void> {
    logger.info('Starting task', { taskId })

    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Check if last iteration has status 'created'
    const lastIteration = task.iterations[task.iterations.length - 1]

    if (!lastIteration || lastIteration.status !== 'created') {
      throw new ValidationError(
        'Last iteration does not have status "created". Cannot start task.',
        { status: 'Last iteration not in created state' },
      )
    }

    // Wait for workspace to be ready
    const maxWaitTime = TaskService.WORKSPACE_READY_TIMEOUT
    const checkInterval = TaskService.WORKSPACE_CHECK_INTERVAL
    const startWait = Date.now()

    while (Date.now() - startWait < maxWaitTime) {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (workspace && workspace.getStatus() === 'idle') {
        break
      }
      await new Promise((resolve) => setTimeout(resolve, checkInterval))
    }

    // Update task status to running
    const previousStatus = task.status
    task.status = 'running'

    // Update iteration status to active
    lastIteration.status = 'active'

    // Save task with updated statuses
    await this.storage.saveTask(task)

    // Emit status change event
    await this.eventService.emitEvent({
      taskId,
      type: EventType.TASK_STATUS,
      subType: 'running',
      payload: {
        status: 'running',
        previousStatus,
        iterationNumber: lastIteration.iterationNumber,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      agentType: task.agentType,
      isError: false,
    })

    // Execute the agent with the iteration's initial message
    if (lastIteration.initialMessage || lastIteration.commandName) {
      const executeTask = this.agentService
        .executeAgent(
          taskId,
          lastIteration.initialMessage || '',
          lastIteration,
          'claude-code',
          undefined,
          lastIteration.model,
        )
        .catch((error) => {
          logger.error('Failed to execute agent during task start', error as Error, {
            taskId,
            iterationNumber: lastIteration.iterationNumber,
          })
          // Update status back to open on error
          task.status = 'open'
          lastIteration.status = 'error'
          this.storage.saveTask(task).catch((saveError) => {
            logger.error('Failed to update task status after agent error', saveError as Error)
          })
        })

      // Track the background agent execution
      this.workspaceService.trackBackgroundTask(taskId, executeTask)
    }

    logger.info('Task started successfully', {
      taskId,
      iterationNumber: lastIteration.iterationNumber,
    })
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
          serverStartCommand: 'pnpm run dev',
          serverPort: 3000,
          injectionStrategy: {
            method: 'html-injection',
          },
        },
      ),
    ]

    // Find the template
    const template = templates.find((t) => t.id === params.templateId)
    if (!template) {
      throw new ValidationError(`Template with id ${params.templateId} not found`, 'templateId')
    }

    // Create a directory for cloning the template
    const dataDir = getDataDir()
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
      useGitWorktrees: false,
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

    const task = await this.createTask(createTaskParams)
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

  async listTasks(
    limit = 50,
    offset = 0,
    status?: string,
  ): Promise<{ tasks: Task[]; total: number }> {
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

    // Store total count before pagination
    const total = tasks.length

    // Apply pagination
    const paginatedTasks = tasks.slice(offset, offset + limit)

    return { tasks: paginatedTasks, total }
  }

  /**
   * Creates a new task iteration without starting it
   */
  private async createTaskIteration(
    task: Task,
    message?: string,
    commandName?: string,
    model?: string,
    fileIds?: string[],
    filePaths?: string[],
    initialStatus: 'created' | 'active' = 'active',
  ): Promise<TaskIteration> {
    // Validate that either message or commandName is provided
    if (!message && !commandName) {
      throw new ValidationError(
        'Either initialMessage or commandName must be provided for task iteration',
        { message: 'Missing required message or command' },
      )
    }

    const now = new Date()
    const iterationNumber = task.iterations.length + 1

    const iteration: TaskIteration = {
      iterationNumber,
      startTime: now,
      status: initialStatus,
      messageCount: 1,
      costUsd: 0,
      initialMessage: message,
      commandName,
      model,
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
        const fileMetadata = await this.storage.get<FileMetadata>(
          `uploads/${task.taskId}/${fileId}`,
        )
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

    return iteration
  }

  async sendCommand(
    taskId: string,
    commandName: string,
    resume = true,
    model?: string,
    filePaths?: string[],
  ): Promise<void> {
    const command = await this.commandService.getCommand(commandName)
    if (!command) {
      throw new ValidationError(`Command '${commandName}' not found`, 'commandName')
    }

    // Use the command's prompt as the message
    await this.sendMessage(taskId, command.prompt, resume, model, undefined, filePaths, commandName)
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

    // Mark last iteration as willNotResume if resume is false
    if (!resume && metadata.iterations.length > 0) {
      const lastIteration = metadata.iterations[metadata.iterations.length - 1]
      if (!lastIteration.willNotResume) {
        lastIteration.willNotResume = true
        await this.storage.saveTask(metadata)

        logger.info('Marked last iteration as will not resume due to resume=false', {
          taskId: metadata.taskId,
          iterationNumber: lastIteration.iterationNumber,
        })
      }
    }

    // Create the iteration with 'created' status (this validates inputs)
    const iteration = await this.createTaskIteration(
      metadata,
      message,
      commandName,
      model,
      fileIds,
      filePaths,
      'created',
    )

    // Check if command has willNotResume flag set
    if (commandName) {
      const command = await this.commandService.getCommand(commandName)
      if (command?.willNotResume) {
        iteration.willNotResume = true
        logger.info('Marked iteration as will not resume due to command flag', {
          taskId: metadata.taskId,
          commandName,
          iterationNumber: iteration.iterationNumber,
        })
      }
    }

    // Find the previous iteration's session ID for resume
    let resumeSessionId: string | undefined
    let prependedResult: string | undefined
    if (metadata.iterations.length > 0) {
      const previousIteration = metadata.iterations[metadata.iterations.length - 1]

      // Check if previous iteration has willNotResume flag set
      if (previousIteration.willNotResume && previousIteration.result) {
        // Prepend the result to the message instead of resuming
        prependedResult = previousIteration.result
        resumeSessionId = undefined
      } else if (resume) {
        // Normal resume behavior
        resumeSessionId = previousIteration.agentSessionId
      }
    }

    // Update metadata
    const now = new Date()
    metadata.iterations.push(iteration)
    metadata.lastMessageAt = now
    metadata.numTurns += 1

    // Save task with the new iteration in 'created' status
    await this.storage.saveTask(metadata)

    // Now start the iteration by updating its status to 'active' and starting execution
    const previousStatus = metadata.status
    metadata.status = 'running'
    iteration.status = 'active'

    // Save task with updated statuses
    await this.storage.saveTask(metadata)

    // Emit status change event
    await this.eventService.emitEvent({
      taskId,
      type: EventType.TASK_STATUS,
      subType: 'running',
      payload: {
        status: 'running',
        previousStatus,
        iterationNumber: iteration.iterationNumber,
      },
      eventId: randomUUID(),
      timestamp: now,
      agentType: metadata.agentType,
      isError: false,
    })

    // Pass the full iteration object with task reference
    iteration.task = metadata
    iteration.taskId = taskId

    // Prepend result if needed
    const finalMessage = prependedResult
      ? `Previous result:\n${prependedResult}\n\n${message}`
      : message

    // Execute agent through AgentService in background
    const backgroundTask = this.agentService
      .executeAgent(taskId, finalMessage, iteration, 'claude-code', resumeSessionId, model)
      .catch((error) => {
        logger.error('Background agent execution failed', error as Error, { taskId })
        // Update status back on error
        metadata.status = 'open'
        iteration.status = 'error'
        this.storage.saveTask(metadata).catch((saveError) => {
          logger.error('Failed to update task status after agent error', saveError as Error)
        })
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
      type: EventType.TASK_STATUS,
      subType: 'closed',
      payload: {
        status: 'closed',
        previousStatus,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      agentType: task.agentType,
      isError: false,
    })

    logger.info('Task closed successfully', { taskId })
  }

  async getAvailableCommands(taskId: string): Promise<Command[]> {
    const task = await this.getTask(taskId)

    // Get all commands
    const allCommands = await this.commandService.listCommands()

    // Get workspace status to check for uncommitted changes
    const workspaceStatus = await this.workspaceService.getWorkspaceStatus(taskId)
    const gitStatus = (workspaceStatus as any).gitStatus
    const workspaceHasChanges = gitStatus?.hasUncommittedChanges || false

    // Get last iteration
    const lastIteration =
      task.iterations && task.iterations.length > 0
        ? task.iterations[task.iterations.length - 1]
        : undefined

    // Filter commands based on current context
    const context = {
      task,
      lastIteration,
      workspaceHasChanges,
      gitStatus,
    }

    return filterAvailableCommands(allCommands, context)
  }

  async markIterationAsWillNotResume(taskId: string, iterationNumber: number): Promise<void> {
    const task = await this.getTask(taskId)

    const iterationIndex = task.iterations.findIndex((it) => it.iterationNumber === iterationNumber)

    if (iterationIndex >= 0) {
      task.iterations[iterationIndex].willNotResume = true
      await this.storage.saveTask(task)

      logger.info('Marked iteration as will not resume', {
        taskId,
        iterationNumber,
      })
    } else {
      throw new ValidationError(
        `Iteration ${iterationNumber} not found for task ${taskId}`,
        'iterationNumber',
      )
    }
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
  return new TaskService(
    storage,
    eventService,
    workspaceService,
    agentService,
    commandService,
    repositoryService,
  )
}
