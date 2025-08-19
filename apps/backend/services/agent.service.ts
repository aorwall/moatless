import { randomUUID } from 'node:crypto'
import type { Event, TaskIteration } from '@moatless/types'

import { logger } from '../utils/logger.js'
import type { RepositoryService } from './repository.service.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

export interface AgentExecutionRequest {
  taskId: string
  message: string
  iteration: TaskIteration
  agentType?: string
  resumeSessionId?: string
  model?: string
}

export interface AgentStatus {
  isRunning: boolean
  sessionId?: string
  taskId: string
}

export class AgentService {
  constructor(
    private workspaceService: WorkspaceService,
    private repositoryService: RepositoryService,
    private storage: StorageBackend,
  ) {}

  /**
   * Execute an agent via environment
   */
  async *executeAgent(request: AgentExecutionRequest): AsyncGenerator<Event> {
    const agentLogger = logger.child({
      taskId: request.taskId,
      iterationNumber: request.iteration.iterationNumber,
      component: 'AgentService',
    })

    const {
      taskId,
      message,
      iteration,
      agentType = 'claude-code',
      resumeSessionId,
      model,
    } = request

    // Get workspace and validate state
    agentLogger.debug('Getting workspace for agent execution', { taskId })
    const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
    const status = workspace.getStatus()

    if (status === 'initializing') {
      throw new Error('Workspace is still setting up')
    }

    if (status === 'error') {
      const environment = workspace.getEnvironment()
      const errorMessage = environment.getError() || 'Environment setup failed'
      throw new Error(`Cannot start agent - environment setup failed: ${errorMessage}`)
    }

    // Check if workspace path actually exists
    const isWorkspaceValid = await workspace.isWorkspacePathValid()
    if (!isWorkspaceValid) {
      throw new Error('Workspace path is invalid - workspace needs to be recreated')
    }

    agentLogger.debug('Getting environment from workspace', {
      agentType,
      workspaceStatus: status,
      hasResumeSession: !!resumeSessionId,
      model,
    })

    // Get environment from workspace
    const environment = workspace.getEnvironment()

    // Prepare message with file attachments if provided
    const finalMessage = await this.prepareMessageWithAttachments(message, iteration, agentLogger)

    try {
      // Check if preview server is available for this task
      const hasPreviewServer = await this.checkPreviewServerAvailability(taskId, agentLogger)

      // Generate system prompt parts based on task state
      const systemPromptParts = await this.generateSystemPromptParts(
        taskId,
        iteration,
        hasPreviewServer,
        agentLogger,
      )

      // Run agent using environment's runAgent method
      const runOptions = {
        workspaceDir: workspace.getWorkspacePath(),
        iterationNumber: iteration.iterationNumber,
        resume: resumeSessionId,
        model,
        agentMode: iteration.agentMode,
        hasPreviewServer,
        appendSystemPrompt:
          systemPromptParts.length > 0 ? systemPromptParts.join('\n\n') : undefined,
      }

      agentLogger.info('Starting agent execution with options', { ...runOptions })

      // Process agent events using environment's runAgent method
      for await (const event of environment.runAgent(agentType, finalMessage, runOptions)) {
        // Capture session ID if it's not already set
        const agentStatus = environment.getAgentStatus()
        if (!iteration.agentSessionId && agentStatus.sessionId) {
          iteration.agentSessionId = agentStatus.sessionId
          agentLogger.debug('Captured agent session ID', { sessionId: iteration.agentSessionId })
        }

        if (event.type === 'agent' && event.subType === 'completed') {
          const resultPayload = event.payload
          agentLogger.info('Agent execution completed', {
            costUsd: resultPayload.total_cost_usd,
            hasResult: resultPayload.subtype === 'success' && 'result' in resultPayload,
            isError: event.isError,
            subtype: resultPayload.subtype,
          })

          // Set basic iteration data for local tracking
          iteration.costUsd = resultPayload.total_cost_usd ?? 0
          iteration.endTime = new Date()
        }

        // Emit agent events to event service with agentSessionId
        agentLogger.debug('Emitting agent event to event service', { eventType: event.type })
        yield {
          ...event,
          agentSessionId: iteration.agentSessionId,
        }
      }

      agentLogger.info('Agent execution completed successfully', {
        iterationNumber: iteration.iterationNumber,
        status: iteration.status,
      })
    } catch (error) {
      agentLogger.error('Agent execution failed', error as Error)

      // Set basic iteration data for local tracking
      iteration.status = 'failed'
      iteration.result = `Agent execution failed: ${error instanceof Error ? error.message : String(error)}. Agent: ${agentType}, Model: ${model || 'default'}, Resuming: ${!!resumeSessionId}`
      iteration.endTime = new Date()

      // Emit error event
      yield {
        taskId,
        type: 'agent',
        subType: 'error',
        payload: {
          error: error instanceof Error ? error.message : String(error),
          stackTrace: error instanceof Error ? error.stack : undefined,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        agentType: 'claude-code',
        isError: true,
        iterationId: iteration.iterationNumber,
        agentSessionId: iteration.agentSessionId,
      }

      throw error
    }
  }

  /**
   * Get agent status for a task
   */
  async getAgentStatus(taskId: string): Promise<AgentStatus> {
    try {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (!workspace) {
        return { isRunning: false, taskId }
      }

      const environment = workspace.getEnvironment()
      const agentStatus = environment.getAgentStatus()

      return {
        isRunning: agentStatus.state === 'running',
        sessionId: agentStatus.sessionId,
        taskId,
      }
    } catch (error) {
      logger.warn('Failed to get agent status', {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      })
      return { isRunning: false, taskId }
    }
  }

  /**
   * Stop agent execution for a task
   */
  async stopAgent(taskId: string): Promise<void> {
    try {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (workspace) {
        const environment = workspace.getEnvironment()
        await environment.stopAgent()
      }
    } catch (error) {
      logger.warn('Failed to stop agent', {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  private async prepareMessageWithAttachments(
    message: string,
    iteration: TaskIteration,
    agentLogger: any,
  ): Promise<string> {
    let finalMessage = message

    if (iteration.attachments && iteration.attachments.length > 0) {
      // Get the absolute paths for the uploaded files
      const filePaths: string[] = iteration.attachments.map((attachment) => attachment.path)

      // Log each attachment
      for (const attachment of iteration.attachments) {
        agentLogger.info('File attachment included', {
          filename: attachment.filename,
          path: attachment.path,
          mimeType: attachment.mimeType,
        })
      }

      // Append file information to message
      finalMessage = `${message}\n\nUploaded files:\n${filePaths.join('\n')}`
      agentLogger.info('Message with file attachments prepared', {
        fileCount: filePaths.length,
        messageLength: finalMessage.length,
      })
    }

    return finalMessage
  }

  private async checkPreviewServerAvailability(taskId: string, agentLogger: any): Promise<boolean> {
    try {
      agentLogger.debug('Checking preview server availability', { taskId })

      const task = await this.storage.getTask(taskId)
      if (task) {
        const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
        return Boolean(repository?.previewConfig?.serverStartCommands?.length)
      }

      return false
    } catch (error) {
      agentLogger.warn('Failed to check preview server availability', {
        error: error instanceof Error ? error.message : String(error),
      })
      return false
    }
  }

  private async generateSystemPromptParts(
    taskId: string,
    iteration: TaskIteration,
    hasPreviewServer: boolean,
    agentLogger: any,
  ): Promise<string[]> {
    const systemPromptParts: string[] = []

    // Add task metadata instruction only if name is not set and it's the first iteration
    if (iteration.iterationNumber === 1) {
      try {
        const task = await this.storage.getTask(taskId)
        if (!task?.name) {
          systemPromptParts.push(
            "IMPORTANT: Before starting work on this task, please use the 'mcp__moatless-tools__updateTaskMetadata' tool to set a clear task name and description (max 2 sentences or 25 words) based on what you need to accomplish.",
          )
        }
      } catch (error) {
        agentLogger.warn('Failed to check task name for system prompt', {
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }

    // Add preview server information when available
    if (hasPreviewServer) {
      systemPromptParts.push(
        "**Interface Layout**: On the left hand side of the interface, there's a chat window where users chat with you. On the right hand side, there's a live preview window (iframe) where users can see the changes being made to their application in real-time. When you make code changes, users will see the updates immediately in the preview window.",
        '**CRITICAL - Preview Server Management**: A preview server is running for this task. You MUST use the provided MCP tools to manage the server - DO NOT use bash commands like `npm start`, `yarn dev`, `bun dev`, or any other manual server commands. Use these MCP tools instead:',
        '- `mcp__moatless-tools__startOrRestartPreviewServer` - Start or restart the preview server',
        '- `mcp__moatless-tools__getPreviewServerLogs` - View server logs',
        'These tools ensure proper integration with the preview window. Manual server commands will conflict with the managed preview server.',
      )
    }

    // Add repository management information
    systemPromptParts.push(
      '**Repository Configuration**: You have access to repository management tools:',
      '- `mcp__moatless-tools__install` - Install the repository by running setup commands (e.g., npm install, pip install)',
      '- `mcp__moatless-tools__updateRepositoryConfig` - Update repository configuration (setup commands, environment variables, server start commands)',
      '- `mcp__moatless-tools__getRepositoryConfig` - Get current repository configuration',
      'Use these tools to manage repository installation and configuration instead of manual commands when appropriate.',
    )

    return systemPromptParts
  }
}

export function createAgentService(
  workspaceService: WorkspaceService,
  repositoryService: RepositoryService,
  storage: StorageBackend,
): AgentService {
  return new AgentService(workspaceService, repositoryService, storage)
}
