import path from 'node:path'
import type { Event, MessageContext, PreviewUrlContextData, TaskIteration } from '@moatless/types'
import { generateComponentList } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { Logger } from '../../../utils/logger'
import { logger } from '../../../utils/logger'
import { ClaudeTokenService } from '../../claude/services/claude-token.service'
import { FigmaService } from '../../figma'
import { FileStorageService } from '../../file-storage'
import { PlaywrightInteractionService } from '../../preview/services/playwright-interaction.service'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { TaskIterationService } from '../../tasks/services/task-iteration.service'
import { LocalEnvironmentDriver } from '../../workspace/drivers/local-environment-driver'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type { AgentRunOptions } from '../agent/base-agent'
import { MessageService } from './message.service'

/**
 * AgentController - Manages agent lifecycle
 *
 * Responsibilities:
 * - Orchestrate agent start/stop
 * - Update runtime state via RuntimeManager
 * - Delegate actual execution to environment drivers
 * - Save agent messages during execution
 * - Reset Playwright mode when agent completes
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
    logger.debug('AgentController: Starting agent', { taskId, agentType })

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
      // 5. Save initial user message if iteration number is provided
      await this.saveInitialUserMessage(taskId, message, runOptions)

      // 6. Prepare message with files and contexts
      const { message: preparedMessage, contentBlocks } = await this.prepareMessageWithAttachments(
        taskId,
        message,
        runOptions,
        agentLogger,
      )

      // 7. Build system prompt and run options
      const finalRunOptions = await this.buildRunOptions(
        taskId,
        runOptions,
        iteration,
        effectiveRepositoryId,
        agentLogger,
      )

      // 8. Get workspace directory
      const workspaceDir = await this.getWorkspaceDirectory(
        effectiveRepositoryId,
        runOptions,
        handle.workspacePath,
      )

      // 8.5. Fetch Claude token before creating agent
      const claudeToken = await this.getClaudeToken(agentLogger)

      // 9. Create agent via driver with token (driver stores in its map)
      const agentOptions = {
        ...(runOptions?.model && { model: runOptions.model }),
        ...(claudeToken && { apiKey: claudeToken }),
      }
      const agent = this.driver.createAgent(taskId, agentType, agentOptions)

      // 10. Run agent and stream enriched events
      const agentStream = agent.run(preparedMessage, {
        ...finalRunOptions,
        workspaceDir,
        contentBlocks, // Pass all content blocks (images + text files) to agent
      })

      yield* this.processAgentStream(agentStream, taskId, undefined, iteration, agentLogger)

      // 11. Agent completed successfully - get captured sessionId
      const capturedSessionId = iteration?.agentSessionId || (await this.getAgentSessionId(taskId))
      logger.debug('AgentController: Agent completed', {
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

      // 13. Reset Playwright mode to idle (return control to user)
      await this.playwrightService.setMode(taskId, 'idle')
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

      // Reset Playwright mode to idle (return control to user)
      await this.playwrightService.setMode(taskId, 'idle')

      // Mark active iteration as completed with "Stopped by user"
      await this.taskIterationService.markIterationAsCompleted(taskId, 'Stopped by user')

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
   */
  private async prepareMessageWithAttachments(
    taskId: string,
    message: string,
    runOptions: AgentRunOptions | undefined,
    agentLogger: Logger,
  ): Promise<{ message: string; contentBlocks: import('@moatless/types').UserContent[] }> {
    // Load all uploaded files (images + text) as content blocks
    // Throws error if unsupported file types (PDFs, binaries)
    const uploadedContentBlocks = await this.loadAllFileAttachments(
      runOptions?.fileIds,
      agentLogger,
    )

    // Append contexts to message and collect Figma image content blocks
    const { message: preparedMessage, contentBlocks: contextContentBlocks } =
      await this.appendContextsToMessage(taskId, message, runOptions?.contexts, agentLogger)

    // Merge uploaded file content blocks with Figma context content blocks
    const allContentBlocks = [...uploadedContentBlocks, ...contextContentBlocks]

    agentLogger.debug('Message prepared with all content blocks', {
      uploadedContentBlockCount: uploadedContentBlocks.length,
      contextContentBlockCount: contextContentBlocks.length,
      totalContentBlockCount: allContentBlocks.length,
    })

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
   * Load all file attachments as content blocks (images, text files)
   * Rejects unsupported binary files (PDFs, etc.)
   */
  private async loadAllFileAttachments(
    fileIds: string[] | undefined,
    agentLogger: Logger,
  ): Promise<import('@moatless/types').UserContent[]> {
    if (!fileIds || fileIds.length === 0) {
      return []
    }

    const contentBlocks: import('@moatless/types').UserContent[] = []

    for (const fileId of fileIds) {
      try {
        // Get file metadata to determine file type
        const metadata = await this.uploadService.getFileMetadata(fileId)

        // Check for unsupported binary files
        if (this.isBinaryFile(metadata.mimeType)) {
          throw new Error(
            `File type "${metadata.mimeType}" is not supported. Supported: images (png/jpg/gif/webp) and text files (code, json, markdown, etc.). File: ${metadata.filename}`,
          )
        }

        // Process image files
        if (this.isImageFile(metadata.mimeType)) {
          const imageBlock = await this.loadImageAsBase64(fileId)
          contentBlocks.push(imageBlock)
          agentLogger.info('Image loaded as base64 content block', {
            fileId,
            filename: metadata.filename,
            mimeType: metadata.mimeType,
            base64Length: imageBlock.source.data.length,
          })
        }
        // Process text files
        else if (this.isTextFile(metadata.mimeType)) {
          const textBlock = await this.loadTextFileAsContentBlock(fileId)
          contentBlocks.push(textBlock)
          agentLogger.info('Text file loaded as content block', {
            fileId,
            filename: metadata.filename,
            mimeType: metadata.mimeType,
            contentLength: textBlock.text.length,
          })
        }
        // Unsupported file type
        else {
          throw new Error(
            `Unknown file type "${metadata.mimeType}". File: ${metadata.filename}. Please upload images or text-based files.`,
          )
        }
      } catch (error) {
        // Re-throw the error to stop processing and notify the user
        agentLogger.error('Failed to load file attachment', {
          fileId,
          error: error instanceof Error ? error.message : String(error),
        })
        throw error
      }
    }

    agentLogger.info('All file attachments loaded as content blocks', {
      totalFiles: fileIds.length,
      contentBlockCount: contentBlocks.length,
    })

    return contentBlocks
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

      const workspacePath = snapshot.executionTarget.workspacePath

      // Use FigmaService to get context for workspace (returns both text and optional image block)
      const { contextText, imageBlock } = await this.figmaService.getFigmaContextForWorkspace(
        figmaData.importId,
        workspacePath,
      )

      agentLogger.debug('Figma context processed successfully', {
        importId: figmaData.importId,
        hasImageBlock: imageBlock !== null,
      })

      return { contextText, imageBlock }
    } catch (error) {
      agentLogger.warn('Failed to process Figma context', {
        importId: figmaData.importId,
        error: error instanceof Error ? error.message : String(error),
      })
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

      agentLogger.debug('Preview URL context processed successfully', {
        path: previewUrlData.path,
        displayUrl,
      })

      return `User is currently viewing: ${displayUrl}`
    } catch (error) {
      agentLogger.warn('Failed to process Preview URL context', {
        path: previewUrlData.path,
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
   * Check if a MIME type represents an image file
   */
  private isImageFile(mimeType: string): boolean {
    const supportedImageTypes = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']
    return supportedImageTypes.includes(mimeType.toLowerCase())
  }

  /**
   * Check if a MIME type represents a text-based file
   */
  private isTextFile(mimeType: string): boolean {
    const normalizedType = mimeType.toLowerCase()

    // Text/* catch-all
    if (normalizedType.startsWith('text/')) {
      return true
    }

    // Specific text-based application types
    const textBasedTypes = [
      'application/javascript',
      'application/typescript',
      'application/json',
      'application/xml',
      'application/yaml',
      'application/toml',
      'application/xhtml+xml',
    ]

    return textBasedTypes.includes(normalizedType)
  }

  /**
   * Check if a MIME type represents an unsupported binary file (PDF, etc.)
   */
  private isBinaryFile(mimeType: string): boolean {
    const normalizedType = mimeType.toLowerCase()

    // Already supported types
    if (this.isImageFile(mimeType) || this.isTextFile(mimeType)) {
      return false
    }

    // Common binary types to explicitly reject
    const binaryTypes = [
      'application/pdf',
      'application/zip',
      'application/x-rar-compressed',
      'application/octet-stream',
      'application/vnd.', // MS Office files
      'video/',
      'audio/',
    ]

    return binaryTypes.some((type) => normalizedType.startsWith(type))
  }

  /**
   * Load an image file and convert it to a base64-encoded content block
   */
  private async loadImageAsBase64(
    fileId: string,
  ): Promise<import('@moatless/types').ImageContentBlock> {
    const { buffer, metadata } = await this.uploadService.getFile(fileId)

    if (!this.isImageFile(metadata.mimeType)) {
      throw new Error(`File is not a supported image type: ${metadata.mimeType}`)
    }

    const base64Data = buffer.toString('base64')
    const mediaType = metadata.mimeType as 'image/png' | 'image/jpeg' | 'image/gif' | 'image/webp'

    return {
      type: 'image',
      source: {
        type: 'base64',
        media_type: mediaType,
        data: base64Data,
      },
    }
  }

  /**
   * Get language identifier from file extension for markdown code blocks
   */
  private getLanguageFromExtension(filename: string): string {
    const ext = path.extname(filename).toLowerCase()
    const langMap: Record<string, string> = {
      '.ts': 'typescript',
      '.tsx': 'typescript',
      '.js': 'javascript',
      '.jsx': 'javascript',
      '.py': 'python',
      '.json': 'json',
      '.md': 'markdown',
      '.yml': 'yaml',
      '.yaml': 'yaml',
      '.xml': 'xml',
      '.html': 'html',
      '.css': 'css',
      '.sh': 'bash',
      '.bash': 'bash',
      '.sql': 'sql',
      '.rs': 'rust',
      '.go': 'go',
      '.java': 'java',
      '.cpp': 'cpp',
      '.c': 'c',
      '.rb': 'ruby',
      '.php': 'php',
      '.swift': 'swift',
      '.kt': 'kotlin',
    }
    return langMap[ext] || ''
  }

  /**
   * Load a text file and format it as a markdown text content block
   */
  private async loadTextFileAsContentBlock(
    fileId: string,
  ): Promise<import('@moatless/types').TextContent> {
    const { buffer, metadata } = await this.uploadService.getFile(fileId)

    if (!this.isTextFile(metadata.mimeType)) {
      throw new Error(`File is not a supported text type: ${metadata.mimeType}`)
    }

    // Read file content as UTF-8 text
    const content = buffer.toString('utf-8')
    const language = this.getLanguageFromExtension(metadata.filename)

    // Format as markdown with filename header and code block
    const formattedText = `## ${metadata.filename}\n\`\`\`${language}\n${content}\n\`\`\``

    return {
      type: 'text',
      text: formattedText,
    }
  }
}
