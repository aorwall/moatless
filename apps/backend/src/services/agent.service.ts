import { randomUUID } from 'node:crypto'
import type { AgentEvent, Event, TaskIteration } from '@moatless/types'

import { logger } from '../utils/logger.js'
import type { MessageService } from './message.service.js'
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
  sessionId: string | undefined
  taskId: string
}

export class AgentService {
  constructor(
    private workspaceService: WorkspaceService,
    private repositoryService: RepositoryService,
    private storage: StorageBackend,
    private messageService: MessageService,
  ) {}

  /**
   * Execute an agent via environment
   */
  async *executeAgent(request: AgentExecutionRequest): AsyncGenerator<Event> {
    const agentLogger = this.createAgentLogger(request)
    const {
      taskId,
      message,
      iteration,
      agentType = 'claude-code',
      resumeSessionId,
      model,
    } = request

    const { workspace, environment } = await this.prepareWorkspaceAndEnvironment(
      taskId,
      agentLogger,
      agentType,
      resumeSessionId,
      model,
    )

    const finalMessage = await this.prepareMessageWithAttachments(message, iteration, agentLogger)

    try {
      const runOptions = await this.buildRunOptions(
        taskId,
        iteration,
        workspace,
        resumeSessionId,
        model,
        agentLogger,
      )

      await this.saveinitialUserMessage(
        taskId,
        finalMessage,
        resumeSessionId,
        iteration,
        agentLogger,
      )

      agentLogger.info('Starting agent execution with options', { ...runOptions })

      yield* this.processAgentEvents(
        environment,
        agentType,
        finalMessage,
        runOptions,
        taskId,
        iteration,
        agentLogger,
      )

      agentLogger.info('Agent execution completed successfully', {
        iterationNumber: iteration.iterationNumber,
        status: iteration.status,
      })
    } catch (error) {
      yield* this.handleExecutionError(
        error,
        taskId,
        iteration,
        agentType,
        model,
        resumeSessionId,
        agentLogger,
      )
      throw error
    }
  }

  private createAgentLogger(request: AgentExecutionRequest) {
    return logger.child({
      taskId: request.taskId,
      iterationNumber: request.iteration.iterationNumber,
      component: 'AgentService',
    })
  }

  private async prepareWorkspaceAndEnvironment(
    taskId: string,
    agentLogger: any,
    agentType: string,
    resumeSessionId: string | undefined,
    model: string | undefined,
  ) {
    agentLogger.debug('Getting workspace for agent execution', { taskId })
    const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)

    this.validateWorkspaceStatus(workspace)
    await this.validateWorkspacePath(workspace)

    agentLogger.debug('Getting environment from workspace', {
      agentType,
      workspaceStatus: workspace.getStatus(),
      hasResumeSession: Boolean(resumeSessionId),
      model,
    })

    return { workspace, environment: workspace.getEnvironment() }
  }

  private validateWorkspaceStatus(workspace: any): void {
    const status = workspace.getStatus()

    if (status === 'initializing') {
      throw new Error('Workspace is still setting up')
    }

    if (status === 'error') {
      const environment = workspace.getEnvironment()
      const errorMessage = environment.getError() || 'Environment setup failed'
      throw new Error(`Cannot start agent - environment setup failed: ${errorMessage}`)
    }
  }

  private async validateWorkspacePath(workspace: any): Promise<void> {
    const isWorkspaceValid = await workspace.isWorkspacePathValid()
    if (!isWorkspaceValid) {
      throw new Error('Workspace path is invalid - workspace needs to be recreated')
    }
  }

  private async buildRunOptions(
    taskId: string,
    iteration: any,
    workspace: any,
    resumeSessionId: string | undefined,
    model: string | undefined,
    agentLogger: any,
  ) {
    const hasPreviewServer = await this.checkPreviewServerAvailability(taskId, agentLogger)
    const systemPromptParts = await this.generateSystemPromptParts(
      taskId,
      iteration,
      hasPreviewServer,
      agentLogger,
    )

    return {
      workspaceDir: workspace.getWorkspacePath(),
      iterationNumber: iteration.iterationNumber,
      ...(resumeSessionId !== undefined && { resume: resumeSessionId }),
      ...(model !== undefined && { model }),
      ...(iteration.agentMode !== undefined && { agentMode: iteration.agentMode }),
      hasPreviewServer,
      ...(systemPromptParts.length > 0 && { appendSystemPrompt: systemPromptParts.join('\n\n') }),
    }
  }

  private async saveinitialUserMessage(
    taskId: string,
    finalMessage: string,
    resumeSessionId: string | undefined,
    iteration: any,
    agentLogger: any,
  ): Promise<void> {
    await this.messageService.saveUserMessage(
      taskId,
      finalMessage,
      resumeSessionId || iteration.agentSessionId,
    )
    agentLogger.debug('Initial user message persisted via MessageService', {
      taskId,
      isResume: Boolean(resumeSessionId),
    })
  }

  private async *processAgentEvents(
    environment: any,
    agentType: string,
    finalMessage: string,
    runOptions: any,
    taskId: string,
    iteration: any,
    agentLogger: any,
  ): AsyncGenerator<Event> {
    for await (const event of environment.runAgent(agentType, finalMessage, runOptions)) {
      this.captureSessionId(environment, iteration, agentLogger)
      this.saveMessageEvent(event, taskId, iteration, agentLogger)
      this.handleCompletionEvent(event, iteration, agentLogger)

      yield this.enrichEventWithSessionId(event, iteration)
    }
  }

  private captureSessionId(environment: any, iteration: any, agentLogger: any): void {
    const agentStatus = environment.getAgentStatus()
    if (!iteration.agentSessionId && agentStatus.sessionId) {
      iteration.agentSessionId = agentStatus.sessionId
      agentLogger.debug('Captured agent session ID', { sessionId: iteration.agentSessionId })
    }
  }

  private saveMessageEvent(event: any, taskId: string, iteration: any, agentLogger: any): void {
    if (event.type === 'agent' && event.subType === 'message') {
      this.messageService
        .saveAgentMessage(taskId, {
          ...event,
          agentSessionId: iteration.agentSessionId,
        })
        .catch((error) => {
          agentLogger.debug('Message persistence completed with warning', {
            eventId: event.eventId,
            error: error instanceof Error ? error.message : String(error),
          })
        })
    }
  }

  private handleCompletionEvent(event: any, iteration: any, agentLogger: any): void {
    if (event.type === 'agent' && event.subType === 'completed') {
      const resultPayload = event.payload
      agentLogger.info('Agent execution completed', {
        costUsd: resultPayload.total_cost_usd,
        hasResult: resultPayload.subtype === 'success' && 'result' in resultPayload,
        isError: event.isError,
        subtype: resultPayload.subtype,
      })

      iteration.costUsd = resultPayload.total_cost_usd ?? 0
      iteration.endTime = new Date()
    }
  }

  private enrichEventWithSessionId(event: any, iteration: any): Event {
    return {
      ...event,
      payload: {
        ...event.payload,
        agentSessionId: iteration.agentSessionId,
      },
    }
  }

  private *handleExecutionError(
    error: unknown,
    taskId: string,
    iteration: any,
    agentType: string,
    model: string | undefined,
    resumeSessionId: string | undefined,
    agentLogger: any,
  ): Generator<AgentEvent> {
    agentLogger.error('Agent execution failed', error as Error)

    iteration.status = 'failed'
    iteration.result = `Agent execution failed: ${error instanceof Error ? error.message : String(error)}. Agent: ${agentType}, Model: ${model || 'default'}, Resuming: ${Boolean(resumeSessionId)}`
    iteration.endTime = new Date()

    yield {
      taskId,
      type: 'agent',
      subType: 'error',
      agentType: 'claude-code',
      iterationId: iteration.iterationNumber,
      payload: {
        error: error instanceof Error ? error.message : String(error),
        stackTrace: error instanceof Error ? error.stack : undefined,
        agentSessionId: iteration.agentSessionId,
      },
      eventId: randomUUID(),
      timestamp: new Date(),
      isError: true,
    } as AgentEvent
  }

  /**
   * Get agent status for a task
   */
  async getAgentStatus(taskId: string): Promise<AgentStatus> {
    try {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (!workspace) {
        return { isRunning: false, sessionId: undefined, taskId }
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
      return { isRunning: false, sessionId: undefined, taskId }
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
        return (repository?.previewConfig?.serverStartCommands?.length ?? 0) > 0
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

    await this.addTaskMetadataPrompt(taskId, iteration, systemPromptParts, agentLogger)
    this.addPreviewServerPrompt(hasPreviewServer, systemPromptParts)
    await this.addSetupCommandsPrompt(taskId, systemPromptParts, agentLogger)
    this.addRepositoryManagementPrompt(systemPromptParts)

    return systemPromptParts
  }

  private async addTaskMetadataPrompt(
    taskId: string,
    iteration: TaskIteration,
    systemPromptParts: string[],
    agentLogger: any,
  ): Promise<void> {
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
  }

  private addPreviewServerPrompt(hasPreviewServer: boolean, systemPromptParts: string[]): void {
    if (hasPreviewServer) {
      systemPromptParts.push(
        "**Interface Layout**: On the left hand side of the interface, there's a chat window where users chat with you. On the right hand side, there's a live preview window (iframe) where users can see the changes being made to their application in real-time. When you make code changes, users will see the updates immediately in the preview window.",
        '**CRITICAL - Preview Server Management**: A preview server is running for this task. You MUST use the provided MCP tools to manage the server - DO NOT use bash commands like `npm start`, `yarn dev`, `bun dev`, or any other manual server commands. Use these MCP tools instead:',
        '- `mcp__moatless-tools__startOrRestartPreviewServer` - Start or restart the preview server',
        '- `mcp__moatless-tools__getPreviewServerLogs` - View server logs',
        'These tools ensure proper integration with the preview window. Manual server commands will conflict with the managed preview server.',
      )
    }
  }

  private async addSetupCommandsPrompt(
    taskId: string,
    systemPromptParts: string[],
    agentLogger: any,
  ): Promise<void> {
    try {
      const task = await this.storage.getTask(taskId)
      if (task) {
        const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
        const setupCommands = repository?.codeExecution?.setupCommands
        if (setupCommands && setupCommands.length > 0) {
          systemPromptParts.push(
            '**Repository Setup Commands**: The following setup commands are configured for this repository:',
            ...setupCommands.map((cmd, index) => `  ${index + 1}. \`${cmd}\``),
            '',
            'You should run these commands when needed for the repository to work properly. You can use the `mcp__moatless-tools__install` tool to run all setup commands at once, or run them individually using bash commands.',
          )
        }
      }
    } catch (error) {
      agentLogger.warn('Failed to get setup commands for system prompt', {
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  private addRepositoryManagementPrompt(systemPromptParts: string[]): void {
    systemPromptParts.push(
      '**Repository Configuration**: You have access to repository management tools:',
      '- `mcp__moatless-tools__install` - Install the repository by running setup commands (e.g., npm install, pip install)',
      '- `mcp__moatless-tools__updateRepositoryConfig` - Update repository configuration (setup commands, environment variables, server start commands)',
      '- `mcp__moatless-tools__getRepositoryConfig` - Get current repository configuration',
      'Use these tools to manage repository installation and configuration instead of manual commands when appropriate.',
    )
  }
}

export function createAgentService(
  workspaceService: WorkspaceService,
  repositoryService: RepositoryService,
  storage: StorageBackend,
  messageService: MessageService,
): AgentService {
  return new AgentService(workspaceService, repositoryService, storage, messageService)
}
