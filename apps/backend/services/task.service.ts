import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  Command,
  CreateTaskRequest,
  CreateVibeTaskRequest,
  Event,
  SDKResultMessage,
  Task,
  TaskIteration,
} from '@moatless/types'
import { RepositoryTemplate, TaskNotFoundError, ValidationError } from '@moatless/types'

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
  ) {
    // Subscribe to agent events for reactive task updates
    this.setupEventSubscription()
  }

  /**
   * Setup event subscription for reactive task updates
   */
  private setupEventSubscription(): void {
    this.eventService.subscribeAll((event: Event) => {
      // Only handle events that should trigger task updates
      if (this.shouldHandleEvent(event)) {
        this.handleAgentEvent(event).catch((error) => {
          logger.error('Failed to handle agent event for task update', error as Error, {
            eventId: event.eventId,
            taskId: event.taskId,
            eventType: event.type,
          })
        })
      }
    })
  }

  /**
   * Determine if this event should trigger task updates
   */
  private shouldHandleEvent(event: Event): boolean {
    // Handle events that affect task state
    return (
      event.type === 'result' ||
      event.type === 'error' ||
      (event.type === 'message' && event.contextTokens !== undefined) ||
      event.agentSessionId !== undefined // Handle any event with agentSessionId
    )
  }

  /**
   * Handle agent events reactively to update tasks
   */
  private async handleAgentEvent(event: Event): Promise<void> {
    const taskLogger = logger.child({
      taskId: event.taskId,
      eventType: event.type,
      eventId: event.eventId,
    })

    try {
      const task = await this.storage.getTask(event.taskId)
      if (!task) {
        taskLogger.warn('Task not found for event, skipping update')
        return
      }

      // Find the iteration this event belongs to
      const iteration = event.iterationId
        ? task.iterations.find((it) => it.iterationNumber === event.iterationId)
        : task.iterations[task.iterations.length - 1] // Latest iteration if no iterationId

      if (!iteration) {
        taskLogger.warn('Iteration not found for event, skipping update', {
          iterationId: event.iterationId,
          totalIterations: task.iterations.length,
        })
        return
      }

      // Handle agentSessionId if present and not already set
      if (event.agentSessionId && !iteration.agentSessionId) {
        iteration.agentSessionId = event.agentSessionId
        taskLogger.debug('Updated agent session ID from event', { sessionId: event.agentSessionId })
      }

      // Handle different event types
      switch (event.type) {
        case 'result':
          await this.handleResultEvent(task, iteration, event)
          break
        case 'error':
          await this.handleErrorEvent(task, iteration, event)
          break
        case 'message':
          await this.handleMessageEvent(task, iteration, event)
          break
        default:
          // For any other event type, just save if we updated agentSessionId
          if (event.agentSessionId) {
            await this.storage.saveTask(task)
          }
          break
      }

      taskLogger.debug('Successfully handled agent event for task update')
    } catch (error) {
      taskLogger.error('Failed to handle agent event', error as Error)
      throw error
    }
  }

  /**
   * Handle RESULT events from agent execution
   */
  private async handleResultEvent(
    task: Task,
    iteration: TaskIteration,
    event: Event,
  ): Promise<void> {
    const resultPayload = event.payload as SDKResultMessage
    const taskLogger = logger.child({
      taskId: task.taskId,
      iterationNumber: iteration.iterationNumber,
    })

    taskLogger.info('Handling RESULT event for task update', {
      costUsd: resultPayload.total_cost_usd,
      hasResult: resultPayload.subtype === 'success' && 'result' in resultPayload,
      isError: event.isError,
      subtype: resultPayload.subtype,
    })

    // Update iteration cost
    iteration.costUsd = resultPayload.total_cost_usd ?? 0

    // Check if the result event indicates an error
    if (event.isError || resultPayload.is_error) {
      // Handle error result
      iteration.status = 'failed'

      // Determine error message based on subtype
      let errorMessage = 'Agent execution failed'
      if (resultPayload.subtype === 'error_max_turns') {
        errorMessage = 'Maximum number of turns exceeded'
        // Mark as will not resume when max turns is reached
        iteration.willNotResume = true
      } else if (resultPayload.subtype === 'error_during_execution') {
        errorMessage = 'Error occurred during execution'
      }

      iteration.error = {
        message: errorMessage,
        timestamp: new Date(),
      }

      if ('result' in resultPayload && resultPayload.result !== null) {
        iteration.result = resultPayload.result
      }
    } else {
      // Handle successful result
      iteration.status = 'completed'
      // Type guard to ensure we're dealing with a success result
      if ('result' in resultPayload) {
        // Use nullish coalescing to only replace null/undefined, not empty strings
        iteration.result =
          resultPayload.result ?? 'No result found in Claude Code SDK result message'
      } else {
        iteration.result = 'No result found in Claude Code SDK result message'
      }
    }

    iteration.endTime = new Date()
    iteration.durationMs = iteration.endTime.getTime() - iteration.startTime.getTime()

    // Capture git data when iteration completes
    await this.captureGitDataForIteration(task.taskId, iteration)

    // Update task status based on iteration result
    const previousStatus = task.status
    const newStatus = event.isError || resultPayload.is_error ? 'error' : 'open'
    task.status = newStatus

    // Save task with updated iteration and status
    await this.storage.saveTask(task)

    // Emit task status change event if status actually changed
    if (previousStatus !== newStatus) {
      await this.eventService.emitEvent({
        taskId: task.taskId,
        type: 'task_status',
        subType: newStatus,
        payload: {
          status: newStatus,
          previousStatus,
          iterationNumber: iteration.iterationNumber,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        agentType: task.agentType,
        isError: newStatus === 'error',
      })
    }
  }

  /**
   * Handle ERROR events from agent execution
   */
  private async handleErrorEvent(
    task: Task,
    iteration: TaskIteration,
    event: Event,
  ): Promise<void> {
    const taskLogger = logger.child({
      taskId: task.taskId,
      iterationNumber: iteration.iterationNumber,
    })

    taskLogger.error('Handling ERROR event for task update')

    // Save error to iteration
    iteration.status = 'failed'
    iteration.error = {
      message: (event.payload?.error as string) || 'Agent execution failed',
      stack: event.payload?.stackTrace as string | undefined,
      timestamp: new Date(),
    }
    iteration.endTime = new Date()
    iteration.durationMs = iteration.endTime.getTime() - iteration.startTime.getTime()

    // Capture git data even for failed iterations
    await this.captureGitDataForIteration(task.taskId, iteration)

    // Task status remains unchanged - only iteration status changes

    // Save task with updated iteration
    await this.storage.saveTask(task)

    // Emit iteration status change event
    await this.eventService.emitEvent({
      taskId: task.taskId,
      type: 'task_status',
      subType: 'failed',
      payload: {
        status: 'failed',
        previousStatus: 'active',
        iterationNumber: iteration.iterationNumber,
        error: iteration.error?.message,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      agentType: task.agentType,
      isError: true,
    })
  }

  /**
   * Handle MESSAGE events that contain context token updates
   */
  private async handleMessageEvent(
    task: Task,
    iteration: TaskIteration,
    event: Event,
  ): Promise<void> {
    // Update contextTokens if provided in the event
    if (event.contextTokens !== undefined) {
      iteration.contextTokens = event.contextTokens

      // Save task with updated context tokens
      await this.storage.saveTask(task)

      logger.debug('Updated context tokens from MESSAGE event', {
        taskId: task.taskId,
        iterationNumber: iteration.iterationNumber,
        contextTokens: event.contextTokens,
      })
    }
  }

  /**
   * Capture git status and changes for an iteration
   */
  private async captureGitDataForIteration(
    taskId: string,
    iteration: TaskIteration,
  ): Promise<void> {
    const taskLogger = logger.child({ taskId, iterationNumber: iteration.iterationNumber })

    try {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (!workspace) {
        taskLogger.warn('Workspace not found, skipping git data capture')
        return
      }

      // Get git status for sync state
      iteration.gitStatus = await workspace.getGitStatus()
      taskLogger.debug('Captured git status for iteration', {
        syncStatus: iteration.gitStatus?.status,
        commitsAhead: iteration.gitStatus?.commitsAhead,
        commitsBehind: iteration.gitStatus?.commitsBehind,
        hasUncommittedChanges: iteration.gitStatus?.hasUncommittedChanges,
      })

      // Get git changes for working directory state
      iteration.gitChanges = await workspace.getGitChanges()
      taskLogger.debug('Captured git changes for iteration', {
        currentBranch: iteration.gitChanges?.currentBranch,
        status: iteration.gitChanges?.status,
        additions: iteration.gitChanges?.additions,
        deletions: iteration.gitChanges?.deletions,
        modifications: iteration.gitChanges?.modifications,
      })

      // Get current commit hash
      iteration.commitHash = (await workspace.getCurrentCommitHash()) || undefined
      taskLogger.debug('Captured commit hash for iteration', {
        commitHash: iteration.commitHash,
      })
    } catch (error) {
      taskLogger.warn('Failed to capture git status/changes for iteration', {
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

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
        type: 'task_status',
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

          // Update task status to error when agent fails to start
          try {
            const task = await this.storage.getTask(taskId)
            if (task) {
              // Only update iteration status to failed, not task status
              if (task.iterations.length > 0) {
                const lastIteration = task.iterations[task.iterations.length - 1]
                if (lastIteration.status === 'active') {
                  lastIteration.status = 'failed'
                }
              }

              await this.storage.saveTask(task)

              // Emit error event
              await this.eventService.emitEvent({
                eventId: randomUUID(),
                taskId,
                type: 'task_status',
                subType: 'failed',
                isError: true,
                agentType: task.agentType,
                payload: {
                  status: 'failed',
                  previousStatus: 'active',
                  error: error instanceof Error ? error.message : String(error),
                },
              })
            }
          } catch (updateError) {
            logger.error(
              'Failed to update task status after agent start failure',
              updateError as Error,
              { taskId },
            )
          }
          // Don't throw here - workspace was set up successfully
        }
      }

      logger.info('Background workspace setup completed', { taskId, started: shouldStart })
    } catch (error) {
      logger.error('Background workspace setup failed', error as Error, { taskId })

      // Task status remains unchanged - workspace setup error doesn't affect task status
      try {
        const task = await this.storage.getTask(taskId)
        if (task) {
          const previousStatus = task.status
          // No task status change needed

          // Emit status change event
          await this.eventService.emitEvent({
            eventId: randomUUID(),
            taskId,
            type: 'task_status',
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
      if (workspace && workspace.getStatus() === 'ready') {
        break
      }
      await new Promise((resolve) => setTimeout(resolve, checkInterval))
    }

    // Update iteration status to active
    lastIteration.status = 'active'

    // Save task with updated statuses
    await this.storage.saveTask(task)

    // Emit iteration status change event
    await this.eventService.emitEvent({
      taskId,
      type: 'task_status',
      subType: 'active',
      payload: {
        status: 'active',
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
        .catch(async (error) => {
          logger.error('Failed to execute agent during task start', error as Error, {
            taskId,
            iterationNumber: lastIteration.iterationNumber,
          })

          // Update status to error on agent failure
          try {
            const updatedTask = await this.storage.getTask(taskId)
            if (updatedTask) {
              const updatedIteration = updatedTask.iterations.find(
                (it) => it.iterationNumber === lastIteration.iterationNumber,
              )
              if (updatedIteration) {
                const previousStatus = updatedTask.status
                // Only update iteration status, not task status
                updatedIteration.status = 'failed'

                await this.storage.saveTask(updatedTask)

                // Emit error event
                await this.eventService.emitEvent({
                  eventId: randomUUID(),
                  taskId,
                  type: 'task_status',
                  subType: 'error',
                  isError: true,
                  agentType: updatedTask.agentType,
                  payload: {
                    status: 'error',
                    previousStatus,
                    error: error instanceof Error ? error.message : String(error),
                    iterationNumber: lastIteration.iterationNumber,
                  },
                })
              }
            }
          } catch (saveError) {
            logger.error('Failed to update task status after agent error', saveError as Error)
          }
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
          serverStartCommand: 'bun run dev',
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
        setupCommands: template.setupCommand ? [template.setupCommand] : undefined,
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

    // If task exists and has active iteration, check if agent is still running
    if (task && task.iterations.length > 0) {
      const lastIteration = task.iterations[task.iterations.length - 1]
      if (lastIteration.status === 'active') {
        const isAgentRunning = await this.agentService.getTaskRunningStatus(taskId)

        // If agent is not running, update iteration status to failed
        if (!isAgentRunning) {
          lastIteration.status = 'failed'
          await this.storage.saveTask(task)
        }
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
        // Filter by status if provided, otherwise exclude closed tasks by default
        if (status) {
          // If status is explicitly provided, only include tasks with that status
          if (task.status === status) {
            tasks.push(task)
          }
        } else {
          // If no status filter, exclude closed tasks by default
          if (task.status !== 'closed') {
            tasks.push(task)
          }
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
    iteration.status = 'active'

    // Save task with updated statuses
    await this.storage.saveTask(metadata)

    // Emit iteration status change event
    await this.eventService.emitEvent({
      taskId,
      type: 'task_status',
      subType: 'active',
      payload: {
        status: 'active',
        iterationNumber: iteration.iterationNumber,
      },
      eventId: randomUUID(),
      timestamp: now,
      agentType: metadata.agentType,
      isError: false,
    })

    // Prepend result from last iteration if needed
    const finalMessage = prependedResult
      ? `Previous result:\n${prependedResult}\n\n${message}`
      : message

    // Execute agent through AgentService in background
    const backgroundTask = this.agentService
      .executeAgent(taskId, finalMessage, iteration, 'claude-code', resumeSessionId, model)
      .catch(async (error) => {
        logger.error('Background agent execution failed', error as Error, { taskId })

        // Update status to error on agent failure
        try {
          // Only update iteration status, not task status
          iteration.status = 'failed'
          // Ensure we never set null for error messages
          const errorMessage = error instanceof Error ? error.message : String(error)
          iteration.result = errorMessage || 'Unknown error occurred'

          await this.storage.saveTask(metadata)

          // Emit error event
          await this.eventService.emitEvent({
            eventId: randomUUID(),
            taskId,
            type: 'task_status',
            subType: 'failed',
            isError: true,
            agentType: metadata.agentType,
            payload: {
              status: 'failed',
              previousStatus: 'active',
              error: error instanceof Error ? error.message : String(error),
              iterationNumber: iteration.iterationNumber,
            },
          })
        } catch (saveError) {
          logger.error('Failed to update task status after agent error', saveError as Error)
        }
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

    // Clean up workspace (stops agents, servers, containers, etc.)
    await this.workspaceService.cleanupWorkspace(taskId)

    // Remove worktree since the task is being permanently deleted
    await this.workspaceService.removeWorktree(taskId)

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

    // Remove worktree since the task is being closed
    await this.workspaceService.removeWorktree(taskId)

    // Update task status to closed
    const previousStatus = task.status
    task.status = 'closed'
    await this.storage.saveTask(task)

    // Emit status change event
    await this.eventService.emitEvent({
      taskId,
      type: 'task_status',
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
