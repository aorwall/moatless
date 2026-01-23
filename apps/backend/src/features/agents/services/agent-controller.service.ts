import type { MessageContext, PreviewUrlContextData, Task, TaskIteration } from '@moatless/types'
import { generateComponentList } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { Logger } from 'pino'
import { logger } from '../../../utils/logger'
import { runInBackgroundWithSession } from '../../auth'
import { ClaudeTokenService } from '../../claude/services/claude-token.service'
import { FigmaService } from '../../figma'
import { FileStorageService } from '../../file-storage'
import { agentExecutionTotal } from '../../metrics'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { TaskIterationService } from '../../tasks/services/task-iteration.service'
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
 * - Delegate actual execution to environment drivers (via WorkspaceHandle.driver)
 * - Save agent messages during execution
 * - Reset Playwright mode when agent completes
 *
 * Key principle: Manager orchestrates, RuntimeManager owns state, driver executes operations
 * Driver instances are obtained via WorkspaceHandle from ensureWorkspace().
 */
@injectable()
export class AgentController {
  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(MessageService) private readonly messageService: MessageService,
    @inject(FileStorageService) private readonly uploadService: FileStorageService,
    @inject(FigmaService) private readonly figmaService: FigmaService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(ClaudeTokenService) private readonly claudeTokenService: ClaudeTokenService,
    @inject(TaskIterationService) private readonly taskIterationService: TaskIterationService,
  ) {}

  /**
   * Start agent execution with message preparation.
   * Drains the agent event stream (events are emitted by sandbox via Redis,
   * handled by TaskIterationService subscription).
   *
   * @param taskId - Task ID
   * @param agentType - Type of agent to execute
   * @param message - User message
   * @param runOptions - Agent run options
   * @param iteration - Optional iteration for tracking session/cost
   * @param repositoryId - Optional repository ID for system prompt generation
   */
  async startAgent(
    taskId: string,
    agentType: string,
    message: string,
    runOptions?: AgentRunOptions,
    iteration?: TaskIteration,
    repositoryId?: string,
  ): Promise<void> {
    logger.info({ taskId, agentType }, 'AgentController: Starting agent')

    // 1. Ensure workspace and get enriched handle with runtime state
    const handle = await this.runtimeManager.ensureWorkspace(taskId)
    const effectiveRepositoryId = repositoryId || handle.repositoryId

    // 2. Check if agent is already running
    if (handle.agent.state === 'running') {
      throw new Error(`Agent already running for task: ${taskId}`)
    }

    // 3. Get driver from handle (WorkspaceRuntimeManager creates and caches driver instances)
    const driver = handle.driver

    // 4. Initialize agent session (sessionId will be set when captured from SDK)
    // FIXME: Its not running yet...
    await this.runtimeManager.updateAgentState(taskId, {
      state: 'running',
      agentType,
      sessionId: undefined,
      startedAt: new Date(),
    })

    // 5. Create scoped logger for agent execution
    // FIXME: Just have a regular instnace logger
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
      const {
        message: preparedMessage,
        contentBlocks,
        fileAttachments,
      } = await this.prepareMessageWithAttachments(taskId, message, runOptions, agentLogger)

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
      agentExecutionTotal.labels({ status: 'success', agent_type: agentType }).inc()

      // 12. Run agent
      // Events are emitted by sandbox via Redis and handled by TaskIterationService subscription
      // Include fileAttachments for K8s mode (RemoteAgentProxy will serialize and send to worker)
      agentLogger.debug({ taskId }, 'startAgent: Running agent')
      await agent.run(preparedMessage, {
        ...finalRunOptions,
        workspaceDir,
        contentBlocks, // Pass all content blocks (images + text files) to agent
        fileAttachments, // Pass file attachments with base64 data for K8s worker transfer
      })
    } catch (error) {
      // Track error metrics
      agentExecutionTotal.labels({ status: 'error', agent_type: agentType }).inc()

      logger.error(
        {
          err: error as Error,
          taskId,
          agentType,
        },
        'AgentController: Agent failed',
      )

      throw error
    }
  }

  /**
   * Stop agent execution
   */
  async stopAgent(taskId: string): Promise<void> {
    logger.info({ taskId }, 'AgentController: Stopping agent')

    try {
      // Stop agent via RuntimeManager
      await this.runtimeManager.stopAgentAndInvalidate(taskId)

      // Update state
      await this.runtimeManager.updateAgentState(taskId, {
        state: 'idle',
      })

      // Mark active iteration as completed with "Stopped by user"
      await this.taskIterationService.markIterationAsCompleted(taskId, 'Stopped by user')

      const task = await this.taskRepository.get(taskId)
      if (task && task.iterations.length > 0) {
        const lastIteration = task.iterations[task.iterations.length - 1]
        if (lastIteration) {
          await this.taskIterationService.performPostProcessing(task, lastIteration)
        }
      }

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
    // Per-task drivers are managed by WorkspaceRuntimeManager
    // Individual driver cleanup happens via runtimeManager.stopAgentAndInvalidate()
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
   * Prepare message by loading all file attachments.
   * Files are listed in the message (with paths) for agent to read.
   * Also returns fileAttachments with base64 data for K8s worker pod transfer.
   * Merges uploaded file content blocks with Figma context content blocks.
   */
  private async prepareMessageWithAttachments(
    taskId: string,
    message: string,
    runOptions: AgentRunOptions | undefined,
    agentLogger: Logger,
  ): Promise<{
    message: string
    contentBlocks: import('@moatless/types').UserContent[]
    fileAttachments: import('../agent/base-agent').FileAttachment[]
  }> {
    // Load all uploaded files and get file data for K8s transfer
    const {
      contentBlocks: uploadedContentBlocks,
      fileInfoList,
      fileAttachments,
    } = await this.loadAllFileAttachments(runOptions?.fileIds, agentLogger)

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
        fileAttachmentCount: fileAttachments.length,
      },
      'Message prepared with all content blocks and file attachments',
    )

    return { message: preparedMessage, contentBlocks: allContentBlocks, fileAttachments }
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
   * Save user message to the message repository.
   * This method is public to allow callers to save the message BEFORE spawning background tasks,
   * ensuring GET /messages returns the message immediately after POST completes.
   *
   * @param taskId - Task ID
   * @param message - User message content
   * @param runOptions - Run options containing iteration number and other metadata
   */
  async saveUserMessage(
    taskId: string,
    message: string,
    runOptions: AgentRunOptions | undefined,
  ): Promise<void> {
    if (runOptions?.iterationNumber) {
      await this.messageService.saveUserMessage(
        taskId,
        message,
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
        'User message saved by AgentController',
      )
    }
  }

  /**
   * Run agent in background with synchronous message save.
   *
   * This method:
   * 1. Saves the user message synchronously (before returning)
   * 2. Spawns a background task for agent execution
   * 3. Returns immediately after message is saved
   *
   * This ensures GET /messages returns the message immediately after POST completes.
   *
   * @param taskId - Task ID
   * @param agentType - Type of agent to execute
   * @param message - User message
   * @param runOptions - Run options (iterationNumber, resume, model, etc.)
   * @param iteration - Task iteration for tracking
   * @param repositoryId - Repository ID for system prompt
   * @param task - Full task object for saving updates
   */
  async runAgentInBackground(
    taskId: string,
    agentType: string,
    message: string,
    runOptions: AgentRunOptions,
    iteration: TaskIteration,
    repositoryId: string,
    _task: Task,
  ): Promise<void> {
    // Save user message BEFORE spawning background task (if not already saved)
    // This ensures GET /messages returns the message immediately after POST completes
    if (!runOptions.userMessageSaved) {
      await this.saveUserMessage(taskId, message, runOptions)
    } else {
      logger.debug(
        { taskId },
        'runAgentInBackground: Skipping message save - already saved by caller',
      )
    }

    // Mark that message is already saved to skip duplicate save in startAgent
    const runOptionsWithMessageSaved: AgentRunOptions = {
      ...runOptions,
      userMessageSaved: true,
    }

    // Run in background with session context preserved for access control
    // Agent events are handled by TaskIterationService via EventService subscription
    runInBackgroundWithSession(async () => {
      try {
        logger.debug({ taskId }, 'runAgentInBackground: Starting agent execution')

        // Execute agent (drains stream internally, events handled by TaskIterationService)
        await this.startAgent(
          taskId,
          agentType,
          message,
          runOptionsWithMessageSaved,
          iteration,
          repositoryId,
        )

        logger.info({ taskId }, 'runAgentInBackground: Agent execution completed')
      } catch (error) {
        logger.error({ err: error as Error, taskId }, 'Background agent execution failed')
      }
    })
  }

  /**
   * Save initial user message if iteration number is provided and not already saved
   */
  private async saveInitialUserMessage(
    taskId: string,
    preparedMessage: string,
    runOptions: AgentRunOptions | undefined,
  ): Promise<void> {
    // Skip if already saved by caller (via saveUserMessage)
    if (runOptions?.userMessageSaved) {
      logger.debug({ taskId }, 'Skipping user message save - already saved by caller')
      return
    }

    // Delegate to public method
    await this.saveUserMessage(taskId, preparedMessage, runOptions)
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
   * Load all file attachments and collect metadata for message formatting.
   * Also loads file content as base64 for transfer to worker pods in K8s mode.
   * Returns empty content blocks (files are listed in the message, not sent as content blocks).
   */
  private async loadAllFileAttachments(
    fileIds: string[] | undefined,
    agentLogger: Logger,
  ): Promise<{
    contentBlocks: import('@moatless/types').UserContent[]
    fileInfoList: Array<{ filename: string; mimeType: string; filePath: string }>
    fileAttachments: import('../agent/base-agent').FileAttachment[]
  }> {
    if (!fileIds || fileIds.length === 0) {
      return { contentBlocks: [], fileInfoList: [], fileAttachments: [] }
    }

    const fileInfoList: Array<{ filename: string; mimeType: string; filePath: string }> = []
    const fileAttachments: import('../agent/base-agent').FileAttachment[] = []

    for (const fileId of fileIds) {
      try {
        // Get file metadata and content
        const metadata = await this.uploadService.getFileMetadata(fileId)
        const filePath = this.uploadService.getFilePath(fileId)
        const { buffer } = await this.uploadService.getFile(fileId)

        // Collect file info for message formatting
        fileInfoList.push({
          filename: metadata.filename,
          mimeType: metadata.mimeType,
          filePath,
        })

        // Collect file attachment with base64 data for K8s transfer
        fileAttachments.push({
          fileId: metadata.fileId,
          filename: metadata.filename,
          mimeType: metadata.mimeType,
          data: buffer.toString('base64'),
        })

        agentLogger.debug(
          {
            fileId,
            filename: metadata.filename,
            mimeType: metadata.mimeType,
            filePath,
            dataSize: buffer.length,
          },
          'File attachment loaded with data',
        )
      } catch (error) {
        agentLogger.error(
          {
            fileId,
            error: error instanceof Error ? error.message : String(error),
          },
          'Failed to get file attachment',
        )
        throw error
      }
    }

    agentLogger.info(
      {
        totalFiles: fileIds.length,
        totalAttachments: fileAttachments.length,
      },
      'All file attachments loaded',
    )

    // Return empty content blocks - files are only listed in the message
    return { contentBlocks: [], fileInfoList, fileAttachments }
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
