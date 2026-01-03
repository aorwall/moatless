import type { Event, MessageContext, PreviewUrlContextData, TaskIteration } from '@moatless/types'
import { generateComponentList } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { Logger } from 'pino'
import {
  activeAgents,
  agentCostUsdTotal,
  agentExecutionDurationSeconds,
  agentExecutionTotal,
  agentInputTokensTotal,
  agentOutputTokensTotal,
} from '../../../services/metrics'
import { logger } from '../../../utils/logger'
import { ClaudeTokenService } from '../../claude/services/claude-token.service'
import { FigmaService } from '../../figma'
import { FileStorageService } from '../../file-storage'
import { PlaywrightInteractionService } from '../../preview/services/playwright-interaction.service'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { TaskIterationService } from '../../tasks/services/task-iteration.service'
import { FileGateway } from '../../workspace/services/file-gateway.service'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { getWorkspacePathFromTarget } from '../../workspace/types/workspace-runtime.types'
import type { AgentRunOptions } from '../agent/base-agent'
import { MessageService } from './message.service'

/**
 * AgentController - Manages agent lifecycle
 *
 * Responsibilities:
 * - Orchestrate agent start/stop
 * - Update runtime state via RuntimeManager
 * - Delegate actual execution to environment drivers (via FileGateway)
 * - Save agent messages during execution
 * - Reset Playwright mode when agent completes
 *
 * Key principle: Manager orchestrates, RuntimeManager owns state, driver executes operations
 * Driver instances are created per-task by FileGateway.
 */
@injectable()
export class AgentController {
  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(FileGateway) private readonly fileGateway: FileGateway,
    @inject(MessageService) private readonly messageService: MessageService,
    @inject(FileStorageService) private readonly uploadService: FileStorageService,
    @inject(FigmaService) private readonly figmaService: FigmaService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(PlaywrightInteractionService)
    private readonly playwrightService: PlaywrightInteractionService,
    @inject(ClaudeTokenService) private readonly claudeTokenService: ClaudeTokenService,
    @inject(TaskIterationService) private readonly taskIterationService: TaskIterationService,
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
    logger.debug({ taskId, agentType }, 'AgentController: Starting agent')
    const startTime = Date.now()

    // Track agent starting
    activeAgents.inc()

    // 1. Ensure workspace and get enriched handle with runtime state
    const handle = await this.runtimeManager.ensureWorkspace(taskId)
    const effectiveRepositoryId = repositoryId || handle.repositoryId

    // 2. Check if agent is already running
    if (handle.agent.state === 'running') {
      throw new Error(`Agent already running for task: ${taskId}`)
    }

    // 3. Get driver for this task (FileGateway creates and caches driver instances)
    const driver = await this.fileGateway.getDriver(taskId)

    // 4. Initialize agent session (sessionId will be set when captured from SDK)
    await this.initializeAgentSession(taskId, agentType)

    // 5. Create scoped logger for agent execution
    const agentLogger = iteration
      ? this.createAgentLogger(taskId, iteration.iterationNumber)
      : logger.child({ taskId, component: 'AgentController' })

    // 5.5. Set baseline values for resumed sessions (for delta calculation in metrics)
    if (iteration && runOptions?.resume) {
      await this.setBaselineValuesForResume(taskId, runOptions.resume, iteration, agentLogger)
    }

    try {
      // 6. Save initial user message if iteration number is provided
      await this.saveInitialUserMessage(taskId, message, runOptions)

      // 7. Prepare message with files and contexts
      const { message: preparedMessage, contentBlocks } = await this.prepareMessageWithAttachments(
        taskId,
        message,
        runOptions,
        agentLogger,
      )

      // 8. Build system prompt and run options
      const finalRunOptions = await this.buildRunOptions(
        taskId,
        runOptions,
        iteration,
        effectiveRepositoryId,
        agentLogger,
      )

      // 9. Get workspace directory
      const workspaceDir = await this.getWorkspaceDirectory(
        effectiveRepositoryId,
        runOptions,
        handle.workspacePath,
      )

      // 10. Fetch Claude token before creating agent
      const claudeToken = await this.getClaudeToken(agentLogger)

      // 11. Create agent via driver with token
      const agentOptions = {
        ...(runOptions?.model && { model: runOptions.model }),
        ...(claudeToken && { apiKey: claudeToken }),
      }
      const agent = await driver.createAgent(agentType, agentOptions)

      // 12. Run agent and stream enriched events
      const agentStream = agent.run(preparedMessage, {
        ...finalRunOptions,
        workspaceDir,
        contentBlocks, // Pass all content blocks (images + text files) to agent
      })

      yield* this.processAgentStream(agentStream, taskId, undefined, iteration, agentLogger)

      // 13. Agent completed successfully - get captured sessionId
      const capturedSessionId = iteration?.agentSessionId || (await this.getAgentSessionId(taskId))

      // Track successful completion metrics
      const durationSeconds = (Date.now() - startTime) / 1000
      agentExecutionTotal.labels({ status: 'success', agent_type: agentType }).inc()
      agentExecutionDurationSeconds.labels({ agent_type: agentType }).observe(durationSeconds)

      logger.debug(
        {
          taskId,
          agentType,
          sessionId: capturedSessionId,
        },
        'AgentController: Agent completed',
      )
    } catch (error) {
      // Track error metrics
      agentExecutionTotal.labels({ status: 'error', agent_type: agentType }).inc()
      const durationSeconds = (Date.now() - startTime) / 1000
      agentExecutionDurationSeconds.labels({ agent_type: agentType }).observe(durationSeconds)

      // Get captured sessionId for error logging
      const capturedSessionId = iteration?.agentSessionId || (await this.getAgentSessionId(taskId))
      logger.error(
        {
          err: error as Error,
          taskId,
          agentType,
          sessionId: capturedSessionId,
        },
        'AgentController: Agent failed',
      )

      // Note: Error message is now emitted as an agent message event
      // by the agent itself, so no need to save it separately here

      throw error
    } finally {
      // Track active agents decrement
      activeAgents.dec()
      // 14. Release agent slot (driver handles agent cleanup)
      // Wrap in try/catch to ensure cleanup doesn't leave stale state
      try {
        await driver.stopAgent()
      } catch (stopError) {
        logger.error(
          { err: stopError, taskId },
          'AgentController: Failed to stop agent driver in cleanup',
        )
      }

      // 15. Reset agent state - critical to prevent "already running" false positives
      try {
        await this.runtimeManager.updateAgentState(taskId, {
          state: 'idle',
          sessionId: undefined,
          startedAt: undefined,
        })
      } catch (stateError) {
        logger.error(
          { err: stateError, taskId },
          'AgentController: Failed to reset agent state in cleanup - may cause stale state',
        )
      }

      // 16. Reset Playwright mode to idle (return control to user)
      try {
        await this.playwrightService.setMode(taskId, 'idle')
      } catch (playwrightError) {
        logger.error(
          { err: playwrightError, taskId },
          'AgentController: Failed to reset Playwright mode in cleanup',
        )
      }
    }
  }

  /**
   * Stop agent execution
   */
  async stopAgent(taskId: string): Promise<void> {
    logger.info({ taskId }, 'AgentController: Stopping agent')

    try {
      // Stop agent via FileGateway (handles driver management)
      await this.fileGateway.stopAgentAndInvalidate(taskId)

      // Update state
      await this.runtimeManager.updateAgentState(taskId, {
        state: 'idle',
      })

      // Reset Playwright mode to idle (return control to user)
      await this.playwrightService.setMode(taskId, 'idle')

      // Mark active iteration as completed with "Stopped by user"
      await this.taskIterationService.markIterationAsCompleted(taskId, 'Stopped by user')

      // Track stopped metric (agent_type unknown since we don't have context)
      agentExecutionTotal.labels({ status: 'stopped', agent_type: 'unknown' }).inc()

      logger.info({ taskId }, 'AgentController: Agent stopped')
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'AgentController: Failed to stop agent')
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
   * Cleanup - placeholder for backward compatibility
   * With per-task drivers, agents are stopped when their driver is invalidated
   */
  // biome-ignore lint/suspicious/useAwait: Interface requires async, but no async work needed
  async cleanup(): Promise<void> {
    logger.info('AgentController: Cleanup called (drivers are per-task, no global cleanup needed)')
    // Per-task drivers are managed by FileGateway
    // Individual driver cleanup happens via fileGateway.stopAgentAndInvalidate()
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
   * Get Claude token from database (OAuth/PAT).
   * Agent will handle settings.json fallback if this returns null.
   */
  private async getClaudeToken(agentLogger: Logger): Promise<string | null> {
    const token = await this.claudeTokenService.getTokenForAgent()
    if (token) {
      agentLogger.debug('Got Claude token from database')
    } else {
      agentLogger.debug('No Claude token in database, agent will check settings.json')
    }
    return token
  }

  /**
   * Prepare message by loading all file attachments as content blocks
   * Files are NOT copied to workspace - all sent as content blocks
   * Merges uploaded file content blocks with Figma context content blocks
   * Prepends file list to message for agent context
   */
  private async prepareMessageWithAttachments(
    taskId: string,
    message: string,
    runOptions: AgentRunOptions | undefined,
    agentLogger: Logger,
  ): Promise<{ message: string; contentBlocks: import('@moatless/types').UserContent[] }> {
    // Load all uploaded files (images + text) as content blocks
    // Throws error if unsupported file types (PDFs, binaries)
    const { contentBlocks: uploadedContentBlocks, fileInfoList } =
      await this.loadAllFileAttachments(runOptions?.fileIds, agentLogger)

    // Prepend file list to message if files are attached
    let messageWithFileInfo = message
    if (fileInfoList.length > 0) {
      const fileListHeader = 'Attached Files (you can read these files from the paths below):'
      const fileListItems = fileInfoList
        .map((file) => `- ${file.filename} (${file.mimeType})\n  Path: ${file.filePath}`)
        .join('\n')
      const fileListSection = `${fileListHeader}\n${fileListItems}`

      // Prepend file list to message
      messageWithFileInfo = message ? `${fileListSection}\n\n${message}` : fileListSection

      agentLogger.debug(
        {
          fileCount: fileInfoList.length,
          files: fileInfoList.map((f) => ({ filename: f.filename, path: f.filePath })),
        },
        'File list prepended to message',
      )
    }

    // Append contexts to message and collect Figma image content blocks
    const { message: preparedMessage, contentBlocks: contextContentBlocks } =
      await this.appendContextsToMessage(
        taskId,
        messageWithFileInfo,
        runOptions?.contexts,
        agentLogger,
      )

    // Merge uploaded file content blocks with Figma context content blocks
    const allContentBlocks = [...uploadedContentBlocks, ...contextContentBlocks]

    agentLogger.debug(
      {
        uploadedContentBlockCount: uploadedContentBlocks.length,
        contextContentBlockCount: contextContentBlocks.length,
        totalContentBlockCount: allContentBlocks.length,
        hasFileList: fileInfoList.length > 0,
      },
      'Message prepared with all content blocks',
    )

    return { message: preparedMessage, contentBlocks: allContentBlocks }
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
      logger.debug(
        {
          taskId,
          iterationNumber: runOptions.iterationNumber,
          hasResumeSession: Boolean(runOptions.resume),
          hasAttachments: (runOptions.fileIds?.length ?? 0) > 0,
          hasContexts: (runOptions.contexts?.length ?? 0) > 0,
        },
        'Initial user message saved by AgentController',
      )
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

      agentLogger.debug(
        {
          sessionId: eventSessionId,
        },
        'Captured agent session ID and updated runtime state',
      )

      return eventSessionId
    }
    return undefined
  }

  /**
   * Update iteration metadata from completion event
   * Tracks cost and token usage metrics with delta calculation for resumed sessions
   */
  private updateIterationMetadata(
    event: Event,
    iteration: TaskIteration,
    agentLogger: Logger,
  ): void {
    const resultPayload = event.payload
    const agentType = 'agentType' in event ? (event.agentType as string) : 'unknown'

    // Extract usage data from result payload
    const totalCostUsd = resultPayload.total_cost_usd ?? 0
    const usage = resultPayload.usage as
      | { input_tokens?: number; output_tokens?: number }
      | undefined
    const inputTokens = usage?.input_tokens ?? 0
    const outputTokens = usage?.output_tokens ?? 0

    // Calculate deltas for resumed sessions
    // Baseline values are set when iteration is created for resumed sessions
    const costDelta = totalCostUsd - (iteration.baselineCostUsd ?? 0)
    const inputTokensDelta = inputTokens - (iteration.baselineInputTokens ?? 0)
    const outputTokensDelta = outputTokens - (iteration.baselineOutputTokens ?? 0)

    agentLogger.info(
      {
        costUsd: totalCostUsd,
        costDelta,
        inputTokens,
        inputTokensDelta,
        outputTokens,
        outputTokensDelta,
        hasBaseline: iteration.baselineCostUsd !== undefined,
        hasResult: resultPayload.subtype === 'success' && 'result' in resultPayload,
        isError: event.isError,
        subtype: resultPayload.subtype,
      },
      'Agent execution completed',
    )

    // Update iteration with final values
    iteration.costUsd = totalCostUsd
    iteration.inputTokens = inputTokens
    iteration.outputTokens = outputTokens
    iteration.endTime = new Date()

    // Increment Prometheus metrics by delta amounts
    // This ensures resumed sessions don't double-count previous iterations' usage
    if (costDelta > 0) {
      agentCostUsdTotal.labels({ agent_type: agentType }).inc(costDelta)
    }
    if (inputTokensDelta > 0) {
      agentInputTokensTotal.labels({ agent_type: agentType }).inc(inputTokensDelta)
    }
    if (outputTokensDelta > 0) {
      agentOutputTokensTotal.labels({ agent_type: agentType }).inc(outputTokensDelta)
    }
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
        logger.debug(
          {
            eventId: event.eventId,
            error: error instanceof Error ? error.message : String(error),
          },
          'Message persistence completed with warning',
        )
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
   * Load all file attachments and collect metadata for message formatting
   * All file types are accepted - files are listed in the message but not converted to content blocks
   * Returns empty content blocks and file metadata for message formatting
   */
  private async loadAllFileAttachments(
    fileIds: string[] | undefined,
    agentLogger: Logger,
  ): Promise<{
    contentBlocks: import('@moatless/types').UserContent[]
    fileInfoList: Array<{ filename: string; mimeType: string; filePath: string }>
  }> {
    if (!fileIds || fileIds.length === 0) {
      return { contentBlocks: [], fileInfoList: [] }
    }

    const fileInfoList: Array<{ filename: string; mimeType: string; filePath: string }> = []

    for (const fileId of fileIds) {
      try {
        // Get file metadata and path
        const metadata = await this.uploadService.getFileMetadata(fileId)
        const filePath = this.uploadService.getFilePath(fileId)

        // Collect file info for message formatting
        fileInfoList.push({
          filename: metadata.filename,
          mimeType: metadata.mimeType,
          filePath,
        })

        agentLogger.debug(
          {
            fileId,
            filename: metadata.filename,
            mimeType: metadata.mimeType,
            filePath,
          },
          'File attachment metadata collected',
        )
      } catch (error) {
        agentLogger.error(
          {
            fileId,
            error: error instanceof Error ? error.message : String(error),
          },
          'Failed to get file attachment metadata',
        )
        throw error
      }
    }

    agentLogger.info(
      {
        totalFiles: fileIds.length,
      },
      'All file attachment metadata collected',
    )

    // Return empty content blocks - files are only listed in the message
    return { contentBlocks: [], fileInfoList }
  }

  /**
   * Append context items to message for agent reference
   * Returns both message with text context and array of content blocks (images)
   */
  private async appendContextsToMessage(
    taskId: string,
    message: string,
    contexts: MessageContext[] | undefined,
    agentLogger: Logger,
  ): Promise<{ message: string; contentBlocks: import('@moatless/types').UserContent[] }> {
    if (!contexts || contexts.length === 0) {
      return { message, contentBlocks: [] }
    }

    const contextParts: string[] = []
    const contentBlocks: import('@moatless/types').UserContent[] = []

    for (const context of contexts) {
      const result = await this.processContextItem(taskId, context, agentLogger)
      if (result.text) {
        contextParts.push(result.text)
      }
      if (result.imageBlock) {
        contentBlocks.push(result.imageBlock)
      }
    }

    const finalMessage = this.buildFinalMessage(message, contextParts)
    return { message: finalMessage, contentBlocks }
  }

  /**
   * Process a single context item and return text and optional image block
   */
  private async processContextItem(
    taskId: string,
    context: MessageContext,
    agentLogger: Logger,
  ): Promise<{
    text: string | null
    imageBlock: import('@moatless/types').ImageContentBlock | null
  }> {
    if (context.type === 'element') {
      const componentList = generateComponentList(context.data)
      return { text: componentList, imageBlock: null }
    }

    if (context.type === 'figma') {
      const { contextText, imageBlock } = await this.processFigmaContext(
        taskId,
        context.data,
        agentLogger,
      )
      return { text: contextText, imageBlock }
    }

    if (context.type === 'previewUrl') {
      const previewUrlPart = this.processPreviewUrlContext(context.data, agentLogger)
      return { text: previewUrlPart, imageBlock: null }
    }

    return { text: null, imageBlock: null }
  }

  /**
   * Build final message with context parts
   */
  private buildFinalMessage(message: string, contextParts: string[]): string {
    if (contextParts.length === 0) {
      return message
    }

    const contextSection = `Context:\n${contextParts.join('\n\n')}`
    return message ? `${message}\n\n${contextSection}` : contextSection
  }

  /**
   * Process Figma context and return formatted context text and optional image content block
   */
  private async processFigmaContext(
    taskId: string,
    figmaData: { importId: string; fileName: string; nodeName: string },
    agentLogger: Logger,
  ): Promise<{
    contextText: string | null
    imageBlock: import('@moatless/types').ImageContentBlock | null
  }> {
    try {
      // Get workspace path from coordinator
      const snapshot = await this.runtimeManager.getSnapshot(taskId)
      if (!snapshot) {
        throw new Error('Cannot process Figma context - workspace not found')
      }

      const workspacePath = getWorkspacePathFromTarget(snapshot.executionTarget)

      // Use FigmaService to get context for workspace (returns both text and optional image block)
      const { contextText, imageBlock } = await this.figmaService.getFigmaContextForWorkspace(
        figmaData.importId,
        workspacePath,
      )

      agentLogger.debug(
        {
          importId: figmaData.importId,
          hasImageBlock: imageBlock !== null,
        },
        'Figma context processed successfully',
      )

      return { contextText, imageBlock }
    } catch (error) {
      agentLogger.warn(
        {
          importId: figmaData.importId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to process Figma context',
      )
      return { contextText: null, imageBlock: null }
    }
  }

  /**
   * Process Preview URL context and return formatted context
   */
  private processPreviewUrlContext(
    previewUrlData: PreviewUrlContextData,
    agentLogger: Logger,
  ): string | null {
    // Guard: Ignore if path is not set
    if (!previewUrlData.path) {
      return null
    }

    try {
      const displayUrl = previewUrlData.url || previewUrlData.path

      agentLogger.debug(
        {
          path: previewUrlData.path,
          displayUrl,
        },
        'Preview URL context processed successfully',
      )

      return `User is currently viewing: ${displayUrl}`
    } catch (error) {
      agentLogger.warn(
        {
          path: previewUrlData.path,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to process Preview URL context',
      )
      return null
    }
  }

  private async checkPreviewServerAvailability(
    taskId: string,
    repositoryId: string,
    agentLogger: Logger,
  ): Promise<boolean> {
    try {
      agentLogger.debug({ taskId, repositoryId }, 'Checking preview server availability')

      const repository = await this.repoService.getRepositoryById(repositoryId)
      const hasPreviewServer = (repository?.previewConfig?.serverStartCommands?.length ?? 0) > 0

      agentLogger.info(
        {
          taskId,
          repositoryId,
          hasPreviewServer,
          serverStartCommandsCount: repository?.previewConfig?.serverStartCommands?.length ?? 0,
        },
        'Preview server availability check result',
      )

      return hasPreviewServer
    } catch (error) {
      agentLogger.warn(
        {
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to check preview server availability',
      )
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
      '**CRITICAL - Preview Server Management**: A preview server is running for this task. You MUST use the provided MCP tools to manage the server - DO NOT use bash commands like `npm start`, `yarn dev`, `bun dev`, or any other manual server commands.',
      '',
      '**Server Management Tools**:',
      '- `mcp__moatless-tools__startOrRestartPreviewServer` - Start or restart the preview server',
      '- `mcp__moatless-tools__getPreviewServerLogs` - View server logs',
      '',
      '**Interactive Preview Tools** (Playwright-based):',
      '- `mcp__moatless-tools__navigatePreview` - Navigate to a specific URL or path in the preview',
      '- `mcp__moatless-tools__clickElement` - Click an element using CSS selector',
      '- `mcp__moatless-tools__evaluateJavaScript` - Execute JavaScript in the preview page context',
      '- `mcp__moatless-tools__fillPreviewForm` - Fill form fields with values',
      '- `mcp__moatless-tools__takePreviewSnapshot` - Capture current page state with screenshot',
      '',
      '**Important**: When you use interactive preview tools, the user will see the preview iframe update to match your actions. The preview maintains state across tool calls, so you can navigate, click, and interact just like a real user would. Each tool returns a screenshot showing the result of your action.',
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

  /**
   * Set baseline values for resumed sessions.
   * When resuming a session, the SDK returns cumulative totals.
   * To calculate the delta for this iteration, we need to know the previous cumulative values.
   *
   * @param taskId - Task ID
   * @param resumeSessionId - Session ID being resumed
   * @param iteration - Current iteration to set baseline values on
   * @param agentLogger - Logger for debugging
   */
  private async setBaselineValuesForResume(
    taskId: string,
    resumeSessionId: string,
    iteration: TaskIteration,
    agentLogger: Logger,
  ): Promise<void> {
    try {
      const task = await this.taskRepository.get(taskId)
      if (!task) {
        agentLogger.warn('Task not found when setting baseline values for resume')
        return
      }

      // Find previous iterations with the same session ID
      const previousIterations = task.iterations.filter(
        (it) =>
          it.agentSessionId === resumeSessionId && it.iterationNumber < iteration.iterationNumber,
      )

      if (previousIterations.length === 0) {
        agentLogger.debug(
          { resumeSessionId },
          'No previous iterations found with session ID, baseline values will be 0',
        )
        return
      }

      // Get the most recent previous iteration (highest iteration number)
      const lastPreviousIteration = previousIterations.reduce((latest, current) =>
        current.iterationNumber > latest.iterationNumber ? current : latest,
      )

      // Set baseline values from the last iteration's final values
      // The SDK returns cumulative totals, so the last iteration's values are the baseline
      iteration.baselineCostUsd = lastPreviousIteration.costUsd ?? 0
      iteration.baselineInputTokens = lastPreviousIteration.inputTokens ?? 0
      iteration.baselineOutputTokens = lastPreviousIteration.outputTokens ?? 0

      agentLogger.info(
        {
          resumeSessionId,
          previousIterationNumber: lastPreviousIteration.iterationNumber,
          baselineCostUsd: iteration.baselineCostUsd,
          baselineInputTokens: iteration.baselineInputTokens,
          baselineOutputTokens: iteration.baselineOutputTokens,
        },
        'Set baseline values for resumed session',
      )
    } catch (error) {
      agentLogger.warn(
        { error: error instanceof Error ? error.message : String(error) },
        'Failed to set baseline values for resume, metrics may be inflated',
      )
    }
  }
}
