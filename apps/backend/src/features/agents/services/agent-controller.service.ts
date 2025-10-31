import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { Event, MessageContext, TaskIteration } from '@moatless/types'
import { generateComponentList } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { UploadService } from '../../../services/upload.service'
import type { Logger } from '../../../utils/logger'
import { logger } from '../../../utils/logger'
import { FigmaService } from '../../figma'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { LocalEnvironmentDriver } from '../../workspace/drivers/local-environment-driver'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type { AgentRunOptions } from '../implementations/base-agent'
import { MessageService } from './message.service'

/**
 * AgentController - Manages agent lifecycle
 *
 * Responsibilities:
 * - Orchestrate agent start/stop
 * - Update runtime state via RuntimeManager
 * - Delegate actual execution to environment drivers
 * - Save agent messages during execution
 *
 * Key principle: Manager orchestrates, RuntimeManager owns state, driver executes operations
 * Driver owns agent instances and process lifecycle (agent map lives in driver)
 */
@injectable()
export class AgentController {
  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(LocalEnvironmentDriver) private readonly driver: LocalEnvironmentDriver,
    @inject(MessageService) private readonly messageService: MessageService,
    @inject(UploadService) private readonly uploadService: UploadService,
    @inject(FigmaService) private readonly figmaService: FigmaService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
  ) {}

  /**
   * Start agent execution with message preparation and event enrichment
   *
   * @param taskId - Task ID
   * @param agentType - Type of agent to execute
   * @param message - User message
   * @param runOptions - Agent run options
   * @param iteration - Optional iteration for tracking session/cost
   * @param repositoryId - Optional repository ID for system prompt generation
   */
  async *startAgent(
    taskId: string,
    agentType: string,
    message: string,
    runOptions?: AgentRunOptions,
    iteration?: TaskIteration,
    repositoryId?: string,
  ): AsyncGenerator<Event> {
    logger.info('AgentController: Starting agent', { taskId, agentType })

    // 1. Ensure workspace and get enriched handle with runtime state
    const handle = await this.runtimeManager.ensureWorkspace(taskId)
    const effectiveRepositoryId = repositoryId || handle.repositoryId

    // 2. Check if agent is already running
    if (handle.agent.state === 'running') {
      throw new Error(`Agent already running for task: ${taskId}`)
    }

    // 3. Initialize agent session (sessionId will be set when captured from SDK)
    await this.initializeAgentSession(taskId, agentType)

    // 4. Create scoped logger for agent execution
    const agentLogger = iteration
      ? this.createAgentLogger(taskId, iteration.iterationNumber)
      : logger.child({ taskId, component: 'AgentController' })

    try {
      // 5. Prepare message with files and contexts
      const preparedMessage = await this.prepareMessageWithAttachments(
        taskId,
        message,
        runOptions,
        agentLogger,
      )

      // 6. Build system prompt and run options
      const finalRunOptions = await this.buildRunOptions(
        taskId,
        runOptions,
        iteration,
        effectiveRepositoryId,
        agentLogger,
      )

      // 7. Save initial user message if iteration number is provided
      await this.saveInitialUserMessage(taskId, preparedMessage, runOptions)

      // 8. Get workspace directory
      const workspaceDir = await this.getWorkspaceDirectory(
        effectiveRepositoryId,
        runOptions,
        handle.workspacePath,
      )

      // 9. Create agent via driver (driver stores in its map)
      const agentOptions = runOptions?.model ? { model: runOptions.model } : undefined
      const agent = this.driver.createAgent(taskId, agentType, agentOptions)

      // 10. Run agent and stream enriched events
      const agentStream = agent.run(preparedMessage, {
        ...finalRunOptions,
        workspaceDir,
      })

      yield* this.processAgentStream(agentStream, taskId, undefined, iteration, agentLogger)

      // 11. Agent completed successfully - get captured sessionId
      const capturedSessionId = iteration?.agentSessionId || (await this.getAgentSessionId(taskId))
      logger.info('AgentController: Agent completed', {
        taskId,
        agentType,
        sessionId: capturedSessionId,
      })
    } catch (error) {
      // Get captured sessionId for error logging
      const capturedSessionId = iteration?.agentSessionId || (await this.getAgentSessionId(taskId))
      logger.error('AgentController: Agent failed', error as Error, {
        taskId,
        agentType,
        sessionId: capturedSessionId,
      })

      // Note: Error message is now emitted as an agent message event
      // by the agent itself, so no need to save it separately here

      throw error
    } finally {
      // 12. Release agent slot (driver handles agent cleanup)
      await this.driver.stopAgent(taskId)
      await this.runtimeManager.updateAgentState(taskId, {
        state: 'idle',
      })
    }
  }

  /**
   * Stop agent execution
   */
  async stopAgent(taskId: string): Promise<void> {
    logger.info('AgentController: Stopping agent', { taskId })

    try {
      // Delegate to driver (driver manages agent map)
      await this.driver.stopAgent(taskId)

      // Update state
      await this.runtimeManager.updateAgentState(taskId, {
        state: 'idle',
      })

      logger.info('AgentController: Agent stopped', { taskId })
    } catch (error) {
      logger.error('AgentController: Failed to stop agent', error as Error, { taskId })
      throw error
    }
  }

  /**
   * Get agent status
   */
  async getStatus(taskId: string): Promise<{
    agentType: string | undefined
    state: 'idle' | 'running' | 'none'
    sessionId: string | undefined
  }> {
    const snapshot = await this.runtimeManager.getSnapshot(taskId)
    if (!snapshot?.agent) {
      return {
        agentType: undefined,
        state: 'none',
        sessionId: undefined,
      }
    }

    return {
      agentType: snapshot.agent.agentType,
      state: snapshot.agent.state,
      sessionId: snapshot.agent.sessionId,
    }
  }

  /**
   * Cleanup - stop all active agents via driver
   */
  async cleanup(): Promise<void> {
    logger.info('AgentController: Delegating cleanup to driver')

    // Driver handles cleanup of all agents
    await this.driver.cleanup()
  }

  /**
   * Initialize agent session and update runtime state
   * SessionId will be set later when captured from Agent SDK
   */
  private async initializeAgentSession(taskId: string, agentType: string): Promise<void> {
    await this.runtimeManager.updateAgentState(taskId, {
      state: 'running',
      agentType,
      sessionId: undefined,
      startedAt: new Date(),
    })
  }

  /**
   * Prepare message by copying files to workspace and appending contexts
   */
  private async prepareMessageWithAttachments(
    taskId: string,
    message: string,
    runOptions: AgentRunOptions | undefined,
    agentLogger: Logger,
  ): Promise<string> {
    let preparedMessage = await this.copyFilesToWorkspace(
      taskId,
      message,
      runOptions?.fileIds,
      agentLogger,
    )
    preparedMessage = await this.appendContextsToMessage(
      taskId,
      preparedMessage,
      runOptions?.contexts,
      agentLogger,
    )
    return preparedMessage
  }

  /**
   * Build run options with system prompt if iteration is provided
   */
  private async buildRunOptions(
    taskId: string,
    runOptions: AgentRunOptions | undefined,
    iteration: TaskIteration | undefined,
    effectiveRepositoryId: string,
    agentLogger: Logger,
  ): Promise<AgentRunOptions> {
    const finalRunOptions = { ...runOptions }

    if (iteration && effectiveRepositoryId) {
      const repository = await this.repoService.getRepositoryById(effectiveRepositoryId)
      const hasPreviewServer = await this.checkPreviewServerAvailability(
        taskId,
        effectiveRepositoryId,
        agentLogger,
      )
      const systemPromptParts = await this.generateSystemPromptParts(
        taskId,
        iteration,
        effectiveRepositoryId,
        hasPreviewServer,
        repository?.systemPrompt,
      )

      if (systemPromptParts.length > 0) {
        finalRunOptions.appendSystemPrompt = systemPromptParts.join('\n\n')
      }
      finalRunOptions.hasPreviewServer = hasPreviewServer
    }

    return finalRunOptions
  }

  /**
   * Save initial user message if iteration number is provided
   */
  private async saveInitialUserMessage(
    taskId: string,
    preparedMessage: string,
    runOptions: AgentRunOptions | undefined,
  ): Promise<void> {
    if (runOptions?.iterationNumber) {
      await this.messageService.saveUserMessage(
        taskId,
        preparedMessage,
        runOptions.resume,
        runOptions.iterationNumber,
        runOptions.fileIds,
        runOptions.contexts,
      )
      logger.debug('Initial user message saved by AgentController', {
        taskId,
        iterationNumber: runOptions.iterationNumber,
        hasResumeSession: Boolean(runOptions.resume),
        hasAttachments: (runOptions.fileIds?.length ?? 0) > 0,
        hasContexts: (runOptions.contexts?.length ?? 0) > 0,
      })
    }
  }

  /**
   * Get workspace directory for agent execution
   */
  private async getWorkspaceDirectory(
    effectiveRepositoryId: string,
    runOptions: AgentRunOptions | undefined,
    workspacePath: string,
  ): Promise<string> {
    const repoConfig = await this.repoService.getRepositoryById(effectiveRepositoryId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${effectiveRepositoryId}`)
    }
    return runOptions?.workspaceDir || workspacePath
  }

  /**
   * Process agent stream and enrich events with session information
   */
  private async *processAgentStream(
    agentStream: AsyncGenerator<Event>,
    taskId: string,
    sessionId: string | undefined,
    iteration: TaskIteration | undefined,
    agentLogger: Logger,
  ): AsyncGenerator<Event> {
    let currentSessionId = sessionId
    for await (const event of agentStream) {
      // Capture session ID from agent events and update runtime state
      if (event.type === 'agent' && !currentSessionId) {
        const capturedSessionId = await this.captureAgentSessionId(
          taskId,
          event,
          iteration,
          agentLogger,
        )
        if (capturedSessionId) {
          currentSessionId = capturedSessionId
        }
      }

      // Handle completion events to update iteration metadata
      if (iteration && event.type === 'agent' && event.subType === 'completed') {
        this.updateIterationMetadata(event, iteration, agentLogger)
      }

      // Save message events
      if (event.type === 'agent' && event.subType === 'message') {
        this.saveMessageEvent(taskId, event, currentSessionId, iteration)
      }

      // Enrich event with session ID before yielding
      yield this.enrichEventWithSession(event, currentSessionId, iteration)
    }
  }

  /**
   * Capture agent session ID from event and update runtime state
   */
  private async captureAgentSessionId(
    taskId: string,
    event: Event,
    iteration: TaskIteration | undefined,
    agentLogger: Logger,
  ): Promise<string | undefined> {
    const eventSessionId =
      ('agentSessionId' in event.payload && event.payload.agentSessionId) || undefined
    if (eventSessionId && typeof eventSessionId === 'string') {
      // Store in iteration if provided
      if (iteration) {
        iteration.agentSessionId = eventSessionId
      }

      // Update runtime state with captured Agent SDK sessionId
      await this.runtimeManager.updateAgentState(taskId, {
        sessionId: eventSessionId,
      })

      agentLogger.debug('Captured agent session ID and updated runtime state', {
        sessionId: eventSessionId,
      })

      return eventSessionId
    }
    return undefined
  }

  /**
   * Update iteration metadata from completion event
   */
  private updateIterationMetadata(
    event: Event,
    iteration: TaskIteration,
    agentLogger: Logger,
  ): void {
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

  /**
   * Save message event to message service
   */
  private saveMessageEvent(
    taskId: string,
    event: Event,
    sessionId: string | undefined,
    iteration: TaskIteration | undefined,
  ): void {
    this.messageService
      .saveAgentMessage(taskId, {
        ...event,
        payload: {
          ...event.payload,
          agentSessionId: iteration?.agentSessionId || sessionId,
        },
      })
      .catch((error) => {
        logger.debug('Message persistence completed with warning', {
          eventId: event.eventId,
          error: error instanceof Error ? error.message : String(error),
        })
      })
  }

  /**
   * Enrich event with session ID
   */
  private enrichEventWithSession(
    event: Event,
    sessionId: string | undefined,
    iteration: TaskIteration | undefined,
  ): Event {
    return {
      ...event,
      payload: {
        ...event.payload,
        agentSessionId: iteration?.agentSessionId || sessionId,
      },
    }
  }

  /**
   * Copy uploaded files to workspace so agent can access them
   */
  private async copyFilesToWorkspace(
    taskId: string,
    message: string,
    fileIds: string[] | undefined,
    agentLogger: Logger,
  ): Promise<string> {
    if (!fileIds || fileIds.length === 0) {
      return message
    }

    // Get workspace path from coordinator
    const snapshot = await this.runtimeManager.getSnapshot(taskId)
    if (!snapshot) {
      throw new Error('Cannot copy files - workspace not found')
    }

    const workspacePath = snapshot.executionTarget.workspacePath
    const uploadsDir = path.join(workspacePath, 'uploads')

    // Create uploads directory in workspace
    await fs.mkdir(uploadsDir, { recursive: true })

    const copiedFiles: string[] = []

    for (const fileId of fileIds) {
      try {
        // Get file from upload service
        const { buffer, metadata } = await this.uploadService.getFile(fileId)

        // Copy to workspace uploads directory
        const targetPath = path.join(uploadsDir, metadata.filename)
        await fs.writeFile(targetPath, buffer)

        copiedFiles.push(targetPath)
        agentLogger.info('File copied to workspace', {
          fileId,
          filename: metadata.filename,
          targetPath,
        })
      } catch (error) {
        agentLogger.warn('Failed to copy file to workspace', {
          fileId,
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }

    // Append file paths to message for agent context
    if (copiedFiles.length > 0) {
      const finalMessage = message
        ? `${message}\n\nUploaded files:\n${copiedFiles.join('\n')}`
        : `Uploaded files:\n${copiedFiles.join('\n')}`
      agentLogger.info('Files copied to workspace and message prepared', {
        fileCount: copiedFiles.length,
        messageLength: finalMessage.length,
      })
      return finalMessage
    }

    return message
  }

  /**
   * Append context items to message for agent reference
   */
  private async appendContextsToMessage(
    taskId: string,
    message: string,
    contexts: MessageContext[] | undefined,
    agentLogger: Logger,
  ): Promise<string> {
    if (!contexts || contexts.length === 0) {
      return message
    }

    const contextParts: string[] = []

    for (const context of contexts) {
      if (context.type === 'element') {
        const componentList = generateComponentList(context.data)
        contextParts.push(componentList)
      } else if (context.type === 'figma') {
        // Handle Figma context
        const figmaPart = await this.processFigmaContext(taskId, context.data, agentLogger)
        if (figmaPart) {
          contextParts.push(figmaPart)
        }
      }
      // Future context types can be added here
    }

    if (contextParts.length > 0) {
      if (message) {
        return `${message}\n\nContext:\n${contextParts.join('\n\n')}`
      }
      return `Context:\n${contextParts.join('\n\n')}`
    }

    return message
  }

  /**
   * Process Figma context by copying image to workspace and returning formatted context
   */
  private async processFigmaContext(
    taskId: string,
    figmaData: { importId: string; fileName: string; nodeName: string },
    agentLogger: Logger,
  ): Promise<string | null> {
    try {
      // Get workspace path from coordinator
      const snapshot = await this.runtimeManager.getSnapshot(taskId)
      if (!snapshot) {
        throw new Error('Cannot process Figma context - workspace not found')
      }

      const workspacePath = snapshot.executionTarget.workspacePath

      // Use FigmaService to get context for workspace
      const contextText = await this.figmaService.getFigmaContextForWorkspace(
        figmaData.importId,
        workspacePath,
      )

      agentLogger.debug('Figma context processed successfully', {
        importId: figmaData.importId,
      })

      return contextText
    } catch (error) {
      agentLogger.warn('Failed to process Figma context', {
        importId: figmaData.importId,
        error: error instanceof Error ? error.message : String(error),
      })
      return null
    }
  }

  private async checkPreviewServerAvailability(
    taskId: string,
    repositoryId: string,
    agentLogger: Logger,
  ): Promise<boolean> {
    try {
      agentLogger.debug('Checking preview server availability', { taskId, repositoryId })

      const repository = await this.repoService.getRepositoryById(repositoryId)
      const hasPreviewServer = (repository?.previewConfig?.serverStartCommands?.length ?? 0) > 0

      agentLogger.info('Preview server availability check result', {
        taskId,
        repositoryId,
        hasPreviewServer,
        serverStartCommandsCount: repository?.previewConfig?.serverStartCommands?.length ?? 0,
      })

      return hasPreviewServer
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
    repositoryId: string,
    hasPreviewServer: boolean,
    systemPrompt: string | undefined,
  ): Promise<string[]> {
    const parts = [
      systemPrompt ?? null,
      await this.getTaskMetadataPrompt(taskId, iteration),
      ...this.getPreviewServerPrompt(hasPreviewServer),
      ...(await this.getSetupCommandsPrompt(repositoryId)),
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
      const task = await this.taskRepository.get(taskId)
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

  private async getSetupCommandsPrompt(repositoryId: string): Promise<string[]> {
    try {
      const repository = await this.repoService.getRepositoryById(repositoryId)
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

  private createAgentLogger(taskId: string, iterationNumber: number) {
    return logger.child({
      taskId,
      iterationNumber,
      component: 'AgentController',
    })
  }

  /**
   * Get agent session ID from runtime state
   */
  private async getAgentSessionId(taskId: string): Promise<string | undefined> {
    const snapshot = await this.runtimeManager.getSnapshot(taskId)
    return snapshot?.agent?.sessionId
  }
}
