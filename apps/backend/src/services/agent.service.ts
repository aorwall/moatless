import { randomUUID } from 'node:crypto'
import type { AgentEvent, Event, TaskIteration } from '@moatless/types'

import type { BaseEnvironment } from '../environments/base-environment.js'
import type { StorageBackend } from '../storage/index.js'
import type { Logger } from '../utils/logger.js'
import { logger } from '../utils/logger.js'
import type { Workspace } from '../workspaces/workspace.js'
import type { AgentRunOptions } from './index.js'
import type { MessageService } from './message.service.js'
import type { RepositoryService } from './repository.service.js'
import type { WorkspaceService } from './workspace.service.js'

interface AgentExecutionRequest {
  taskId: string
  message: string
  iteration: TaskIteration
  agentType?: string
  resumeSessionId?: string
  model?: string
  systemPrompt?: string
}

interface AgentStatus {
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
      systemPrompt,
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
        systemPrompt,
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
    agentLogger: Logger,
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

  private validateWorkspaceStatus(workspace: Workspace): void {
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

  private async validateWorkspacePath(workspace: Workspace): Promise<void> {
    const isWorkspaceValid = await workspace.isWorkspacePathValid()
    if (!isWorkspaceValid) {
      throw new Error('Workspace path is invalid - workspace needs to be recreated')
    }
  }

  private async buildRunOptions(
    taskId: string,
    iteration: TaskIteration,
    workspace: Workspace,
    resumeSessionId: string | undefined,
    model: string | undefined,
    systemPrompt: string | undefined,
    agentLogger: Logger,
  ) {
    const hasPreviewServer = await this.checkPreviewServerAvailability(taskId, agentLogger)
    const systemPromptParts = await this.generateSystemPromptParts(
      taskId,
      iteration,
      hasPreviewServer,
      systemPrompt,
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
    iteration: TaskIteration,
    agentLogger: Logger,
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
    environment: BaseEnvironment,
    agentType: string,
    finalMessage: string,
    runOptions: AgentRunOptions,
    taskId: string,
    iteration: TaskIteration,
    agentLogger: Logger,
  ): AsyncGenerator<Event> {
    for await (const event of environment.runAgent(agentType, finalMessage, runOptions)) {
      this.captureSessionId(environment, iteration, agentLogger)
      this.saveMessageEvent(event, taskId, iteration, agentLogger)
      this.handleCompletionEvent(event, iteration, agentLogger)

      yield this.enrichEventWithSessionId(event, iteration)
    }
  }

  private captureSessionId(
    environment: BaseEnvironment,
    iteration: TaskIteration,
    agentLogger: Logger,
  ): void {
    const agentStatus = environment.getAgentStatus()
    if (!iteration.agentSessionId && agentStatus.sessionId) {
      iteration.agentSessionId = agentStatus.sessionId
      agentLogger.debug('Captured agent session ID', { sessionId: iteration.agentSessionId })
    }
  }

  private saveMessageEvent(
    event: Event,
    taskId: string,
    iteration: TaskIteration,
    agentLogger: Logger,
  ): void {
    if (event.type === 'agent' && event.subType === 'message') {
      this.messageService
        .saveAgentMessage(taskId, {
          ...event,
          payload: {
            ...event.payload,
            agentSessionId: iteration.agentSessionId,
          },
        })
        .catch((error) => {
          agentLogger.debug('Message persistence completed with warning', {
            eventId: event.eventId,
            error: error instanceof Error ? error.message : String(error),
          })
        })
    }
  }

  private handleCompletionEvent(event: Event, iteration: TaskIteration, agentLogger: Logger): void {
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

  private enrichEventWithSessionId(event: Event, iteration: TaskIteration): Event {
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
    iteration: TaskIteration,
    agentType: string,
    model: string | undefined,
    resumeSessionId: string | undefined,
    agentLogger: Logger,
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

  private prepareMessageWithAttachments(
    message: string,
    iteration: TaskIteration,
    agentLogger: Logger,
  ): string {
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

  private async checkPreviewServerAvailability(
    taskId: string,
    agentLogger: Logger,
  ): Promise<boolean> {
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
    systemPrompt: string | undefined,
  ): Promise<string[]> {
    const parts = [
      systemPrompt ?? null,
      await this.getTaskMetadataPrompt(taskId, iteration),
      ...this.getPreviewServerPrompt(hasPreviewServer),
      ...(await this.getSetupCommandsPrompt(taskId)),
      ...this.getRepositoryManagementPrompt(),
    ]
    return parts.filter((p): p is string => p !== null)
  }

  private async getTaskMetadataPrompt(
    taskId: string,
    iteration: TaskIteration,
  ): Promise<string | null> {
    if (iteration.iterationNumber !== 1) {
      return null
    }

    try {
      const task = await this.storage.getTask(taskId)
      if (!task?.name) {
        return "IMPORTANT: Before starting work on this task, please use the 'mcp__moatless-tools__updateTaskMetadata' tool to set a clear task name and description (max 2 sentences or 25 words) based on what you need to accomplish."
      }
      return null
    } catch {
      return null
    }
  }

  private getPreviewServerPrompt(hasPreviewServer: boolean): string[] {
    if (!hasPreviewServer) {
      return []
    }

    return [
      "**Interface Layout**: On the left hand side of the interface, there's a chat window where users chat with you. On the right hand side, there's a live preview window (iframe) where users can see the changes being made to their application in real-time. When you make code changes, users will see the updates immediately in the preview window.",
      '**CRITICAL - Preview Server Management**: A preview server is running for this task. You MUST use the provided MCP tools to manage the server - DO NOT use bash commands like `npm start`, `yarn dev`, `bun dev`, or any other manual server commands. Use these MCP tools instead:',
      '- `mcp__moatless-tools__startOrRestartPreviewServer` - Start or restart the preview server',
      '- `mcp__moatless-tools__getPreviewServerLogs` - View server logs',
      'These tools ensure proper integration with the preview window. Manual server commands will conflict with the managed preview server.',
    ]
  }

  private async getSetupCommandsPrompt(taskId: string): Promise<string[]> {
    try {
      const task = await this.storage.getTask(taskId)
      if (!task) {
        return []
      }

      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      const setupCommands = repository?.codeExecution?.setupCommands

      if (!setupCommands || setupCommands.length === 0) {
        return []
      }

      return [
        '**Repository Setup Commands**: The following setup commands are configured for this repository:',
        ...setupCommands.map((cmd, index) => `  ${index + 1}. \`${cmd}\``),
        '',
        'You should run these commands when needed for the repository to work properly. You can use the `mcp__moatless-tools__install` tool to run all setup commands at once, or run them individually using bash commands.',
      ]
    } catch {
      return []
    }
  }

  private getRepositoryManagementPrompt(): string[] {
    return [
      '**Repository Configuration**: You have access to repository management tools:',
      '- `mcp__moatless-tools__install` - Install the repository by running setup commands (e.g., npm install, pip install)',
      '- `mcp__moatless-tools__updateRepositoryConfig` - Update repository configuration (setup commands, environment variables, server start commands)',
      '- `mcp__moatless-tools__getRepositoryConfig` - Get current repository configuration',
      'Use these tools to manage repository installation and configuration instead of manual commands when appropriate.',
    ]
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
