import { randomUUID } from 'node:crypto'
import path from 'node:path'
import { query, type SDKUserMessage } from '@anthropic-ai/claude-agent-sdk'
import type {
  AgentEvent,
  BranchNameAndSummary,
  ClaudeCodeOptions,
  ClaudeCodeSDKMessage,
  Event,
  ImageContentBlock,
  McpServerConfig,
} from '@moatless/types'

import { calculateContextTokens } from '../../../../utils/context-tokens'
import { getDataDir } from '../../../../utils/data-dir'
import { parseBranchNameAndSummary } from '../../../../utils/json-parser'
import { logger } from '../../../../utils/logger'
import { createBranchNamePrompt } from '../../../../utils/prompts'
import { type AgentRunOptions, BaseAgent } from '../base-agent'

// Anthropic SDK message content types (matching the API format)
type MessageContentBlock =
  | { type: 'text'; text: string }
  | {
      type: 'image'
      source: {
        type: 'base64'
        media_type: 'image/png' | 'image/jpeg' | 'image/gif' | 'image/webp'
        data: string
      }
    }

export class ClaudeCodeAgent extends BaseAgent {
  private abortController: AbortController | undefined
  private extractedPlan: string | undefined
  private lastSdkMessage: ClaudeCodeSDKMessage | undefined
  private stderrOutput: string[] = []

  constructor(taskId: string, options: ClaudeCodeOptions = {}) {
    super(taskId, options)
  }

  async *run(message: string, runOptions: AgentRunOptions = {}): AsyncGenerator<Event> {
    this.stopRequested = false
    this.abortController = new AbortController()
    this.iterationNumber = runOptions.iterationNumber
    this.stderrOutput = []

    const agentLogger = this.createLogger(runOptions)
    this.logRunStart(agentLogger, runOptions)

    try {
      const sdkOptions = await this.buildSDKOptions(runOptions)
      this.logSDKOptions(agentLogger, sdkOptions, runOptions)

      let eventCount = 0
      let hasStartedProcessing = false
      let hasEmittedStarted = false

      // Determine prompt format based on presence of images
      const hasImages = runOptions.images && runOptions.images.length > 0
      const promptFormat = hasImages ? 'content blocks with images' : 'simple string'

      agentLogger.debug('Initiating Claude Code SDK query', {
        promptLength: message.length,
        promptFormat,
        imageCount: runOptions.images?.length ?? 0,
        taskId: this.taskId,
      })

      // Create prompt - use AsyncIterable format if images are present
      const prompt = hasImages
        ? this.createSDKMessageStream(message, runOptions.images, randomUUID())
        : message

      for await (const sdkMessage of query({
        prompt,
        options: { ...sdkOptions, abortController: this.abortController },
      })) {
        eventCount++
        this.lastSdkMessage = sdkMessage
        this.logSDKMessage(agentLogger, sdkMessage, eventCount)

        if (!hasEmittedStarted) {
          hasEmittedStarted = true
          yield this.createEventWithContextTokens(
            { type: 'agent', subType: 'started' },
            'agent',
            false,
          )
        }

        if (this.stopRequested) {
          agentLogger.info('Stop requested, breaking from SDK query loop', { eventCount })
          break
        }

        this.captureSessionId(agentLogger, sdkMessage)
        this.extractPlanFromMessage(agentLogger, sdkMessage)

        if (sdkMessage.type !== 'system' || hasStartedProcessing) {
          hasStartedProcessing = true
        }

        if (sdkMessage.type === 'result') {
          yield* this.handleResultMessage(agentLogger, sdkMessage)
        } else {
          yield this.createEventWithContextTokens(sdkMessage, 'agent', false, 'message')
        }
      }

      this.logQueryCompletion(agentLogger, eventCount, hasStartedProcessing)
    } catch (error) {
      yield* this.handleError(agentLogger, error)
    } finally {
      agentLogger.debug('Cleaning up Claude Code agent')
      await this.cleanup()
    }
  }

  async stop(): Promise<void> {
    this.stopRequested = true

    if (this.abortController && !this.abortController.signal.aborted) {
      this.abortController.abort()
    }

    await this.cleanup()
  }

  private createLogger(runOptions: AgentRunOptions) {
    return logger.child({
      taskId: this.taskId,
      iterationNumber: runOptions.iterationNumber,
      component: 'ClaudeCodeAgent',
    })
  }

  private logRunStart(
    agentLogger: ReturnType<typeof logger.child>,
    runOptions: AgentRunOptions,
  ): void {
    agentLogger.info('Starting Claude Code agent execution', {
      taskId: this.taskId,
      iterationNumber: runOptions.iterationNumber,
      hasResumeSession: Boolean(runOptions.resume),
      workspaceDir: runOptions.workspaceDir,
    })
  }

  private logSDKOptions(
    agentLogger: ReturnType<typeof logger.child>,
    sdkOptions: ClaudeCodeOptions,
    runOptions: AgentRunOptions,
  ): void {
    agentLogger.info('Claude Code SDK options configured', {
      model: sdkOptions.model,
      allowedTools: sdkOptions.allowedTools,
      permissionMode: sdkOptions.permissionMode,
      agentMode: runOptions.agentMode,
      cwd: sdkOptions.cwd,
      resume: sdkOptions.resume,
      mcpServerNames: sdkOptions.mcpServers ? Object.keys(sdkOptions.mcpServers) : [],
      systemPromptLength: runOptions.appendSystemPrompt?.length || 0,
    })
  }

  private logSDKMessage(
    agentLogger: ReturnType<typeof logger.child>,
    sdkMessage: ClaudeCodeSDKMessage,
    eventCount: number,
  ): void {
    agentLogger.debug('Received SDK message', { eventCount, sdkMessage })
    const logInfo = this.getSDKMessageLogInfo(sdkMessage)
    agentLogger.info(`SDK Event: type=${sdkMessage.type}${logInfo}`, { eventCount })

    // Check for error content items and log warnings
    this.logErrorContentItems(agentLogger, sdkMessage, eventCount)
  }

  private logErrorContentItems(
    agentLogger: ReturnType<typeof logger.child>,
    sdkMessage: ClaudeCodeSDKMessage,
    eventCount: number,
  ): void {
    if (
      !(
        'message' in sdkMessage &&
        sdkMessage.message &&
        'content' in sdkMessage.message &&
        sdkMessage.message.content
      )
    ) {
      return
    }

    const content = Array.isArray(sdkMessage.message.content)
      ? sdkMessage.message.content
      : [sdkMessage.message.content]

    for (const contentItem of content) {
      if (
        contentItem &&
        typeof contentItem === 'object' &&
        'is_error' in contentItem &&
        contentItem.is_error === true
      ) {
        const contentType = 'type' in contentItem ? (contentItem.type as string) : 'unknown'
        const errorContent =
          'content' in contentItem ? (contentItem.content as string) : 'No error message'
        const toolUseId =
          'tool_use_id' in contentItem ? (contentItem.tool_use_id as string) : undefined

        agentLogger.warn(`SDK message contains error content: ${contentType} error`, {
          eventCount,
          contentType,
          errorContent,
          toolUseId,
          fullContentItem: contentItem,
        })
      }
    }
  }

  private captureSessionId(
    agentLogger: ReturnType<typeof logger.child>,
    sdkMessage: ClaudeCodeSDKMessage,
  ): void {
    if (!this.sessionId && sdkMessage.session_id) {
      this.sessionId = sdkMessage.session_id
      agentLogger.info('Session ID captured from SDK', { sessionId: this.sessionId })
    }
  }

  private extractPlanFromMessage(
    agentLogger: ReturnType<typeof logger.child>,
    sdkMessage: ClaudeCodeSDKMessage,
  ): void {
    if (
      sdkMessage.type === 'assistant' &&
      'message' in sdkMessage &&
      sdkMessage.message &&
      'content' in sdkMessage.message &&
      sdkMessage.message.content
    ) {
      const content = Array.isArray(sdkMessage.message.content)
        ? sdkMessage.message.content
        : [sdkMessage.message.content]

      for (const contentItem of content) {
        if (
          contentItem &&
          typeof contentItem === 'object' &&
          'type' in contentItem &&
          contentItem.type === 'tool_use' &&
          'name' in contentItem &&
          contentItem.name === 'ExitPlanMode' &&
          'input' in contentItem &&
          contentItem.input &&
          typeof contentItem.input === 'object' &&
          'plan' in contentItem.input
        ) {
          this.extractedPlan = contentItem.input.plan as string
          agentLogger.info('ExitPlanMode detected, plan extracted', {
            planLength: this.extractedPlan?.length,
          })
          break
        }
      }
    }
  }

  private *handleResultMessage(
    agentLogger: ReturnType<typeof logger.child>,
    sdkMessage: ClaudeCodeSDKMessage,
  ): Generator<AgentEvent> {
    const isError = 'is_error' in sdkMessage ? sdkMessage.is_error : false
    const subtype = 'subtype' in sdkMessage ? sdkMessage.subtype : undefined
    const hasResult = 'result' in sdkMessage && sdkMessage.result !== null

    agentLogger.info('Received result message from SDK', {
      isError,
      subtype,
      hasExtractedPlan: Boolean(this.extractedPlan),
      hasResult,
      totalCostUsd: 'total_cost_usd' in sdkMessage ? sdkMessage.total_cost_usd : undefined,
    })

    // Log detailed information for error result subtypes
    if (subtype === 'error_max_turns' || subtype === 'error_during_execution') {
      agentLogger.error('SDK returned error result', {
        errorSubtype: subtype,
        numTurns: 'num_turns' in sdkMessage ? sdkMessage.num_turns : undefined,
        durationMs: 'duration_ms' in sdkMessage ? sdkMessage.duration_ms : undefined,
        hasStderrOutput: this.stderrOutput.length > 0,
        stderrLineCount: this.stderrOutput.length,
        stderrPreview:
          this.stderrOutput.length > 0 ? this.stderrOutput.slice(0, 3).join('\n') : undefined,
      })
    }

    this.detectContradictoryErrorStates(agentLogger, isError, subtype, sdkMessage)

    const resultEvent = this.createResultEvent(agentLogger, sdkMessage, isError)
    yield resultEvent
  }

  private detectContradictoryErrorStates(
    agentLogger: ReturnType<typeof logger.child>,
    isError: boolean,
    subtype: string | undefined,
    sdkMessage: ClaudeCodeSDKMessage,
  ): void {
    const isErrorSubtype = subtype?.startsWith('error_')
    if (isErrorSubtype && !isError) {
      agentLogger.warn('Contradictory result state: error subtype with is_error=false', {
        subtype,
        isError,
        fullMessage: JSON.stringify(sdkMessage, null, 2),
      })
    } else if (!isErrorSubtype && isError) {
      agentLogger.warn('Contradictory result state: non-error subtype with is_error=true', {
        subtype,
        isError,
        fullMessage: JSON.stringify(sdkMessage, null, 2),
      })
    }
  }

  private createResultEvent(
    agentLogger: ReturnType<typeof logger.child>,
    sdkMessage: ClaudeCodeSDKMessage,
    isError: boolean,
  ): AgentEvent {
    if (this.extractedPlan) {
      agentLogger.info('Including extracted plan in result event', {
        planLength: this.extractedPlan.length,
      })
      const eventWithPlan = this.createEventWithContextTokens(
        sdkMessage,
        'agent',
        isError,
        'completed',
      ) as AgentEvent
      eventWithPlan.payload = {
        ...eventWithPlan.payload,
        result: this.extractedPlan,
      }
      this.extractedPlan = undefined
      return eventWithPlan
    }

    return this.createEventWithContextTokens(
      sdkMessage,
      'agent',
      isError,
      'completed',
    ) as AgentEvent
  }

  private logQueryCompletion(
    agentLogger: ReturnType<typeof logger.child>,
    eventCount: number,
    hasStartedProcessing: boolean,
  ): void {
    agentLogger.info('SDK query loop completed', {
      totalEvents: eventCount,
      hasStartedProcessing,
      stopRequested: this.stopRequested,
      sessionId: this.sessionId,
    })

    if (eventCount === 1 && !hasStartedProcessing) {
      agentLogger.warn(
        'SDK query loop exited after only init message - this may indicate an authentication or configuration issue',
      )
    }
  }

  private *handleError(
    agentLogger: ReturnType<typeof logger.child>,
    error: unknown,
  ): Generator<AgentEvent> {
    // Check for abort errors from user stop request
    if (this.stopRequested && error instanceof Error) {
      const isAbortError = error.message.includes('aborted by user') && error.name === 'AbortError'

      if (isAbortError) {
        agentLogger.info('Agent stopped gracefully by user request', {
          sessionId: this.sessionId,
          errorMessage: error.message,
        })
        return
      }
    }

    if (error instanceof Error && error.message.includes('Claude Code process exited')) {
      const exitCodeMatch = error.message.match(/code (\d+)/)
      const exitCode = exitCodeMatch?.[1] ? Number.parseInt(exitCodeMatch[1], 10) : null
      const isExpectedExitCode = exitCode === 143 || exitCode === 130

      if (this.stopRequested || isExpectedExitCode) {
        agentLogger.info('Claude Code process exited after stop request', {
          exitCode,
          stopRequested: this.stopRequested,
          sessionId: this.sessionId,
        })
        return
      }

      // Log detailed error information for diagnostics
      this.logDetailedErrorInfo(agentLogger, error, exitCode)

      // Yield stderr as a message event if we have error output
      yield* this.yieldStderrMessageEvent(agentLogger)

      yield this.createEventWithContextTokens(
        {
          type: 'agent',
          subType: 'error',
          error: error.message,
          exitCode,
        },
        'agent',
        true,
      )

      throw error
    }

    this.logDetailedErrorInfo(
      agentLogger,
      error instanceof Error ? error : new Error(String(error)),
      undefined,
    )

    // Yield stderr as a message event if we have error output
    yield* this.yieldStderrMessageEvent(agentLogger)

    yield this.createEventWithContextTokens(
      {
        type: 'agent',
        subType: 'error',
        error: error instanceof Error ? error.message : String(error),
        stackTrace: error instanceof Error ? error.stack : undefined,
      },
      'agent',
      true,
    )

    throw error
  }

  getAgentType(): string {
    return 'claude-code'
  }

  private createEventWithContextTokens(
    payload: ClaudeCodeSDKMessage | Record<string, unknown>,
    type: 'agent',
    isError = false,
    subType?: string,
  ): AgentEvent {
    const contextTokens =
      'type' in payload && 'session_id' in payload
        ? calculateContextTokens(payload as ClaudeCodeSDKMessage)
        : 0

    const sessionId =
      'session_id' in payload ? (payload as ClaudeCodeSDKMessage).session_id : this.sessionId

    return {
      eventId: crypto.randomUUID(),
      agentType: this.getAgentType(),
      type,
      subType: subType || ('subType' in payload ? (payload['subType'] as string) : undefined),
      taskId: this.taskId,
      iterationId: this.iterationNumber,
      timestamp: new Date(),
      isError,
      payload: {
        ...payload,
        contextTokens,
        agentSessionId: sessionId,
      },
    } as AgentEvent
  }

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Diagnostic logging increased complexity slightly; refactoring planned
  private async buildSDKOptions(runOptions: AgentRunOptions): Promise<ClaudeCodeOptions> {
    const options = this.options as ClaudeCodeOptions

    // Get settings via inversify
    const { appContainer } = await import('../../../../container/container.js')
    const { SettingsService } = await import('../../../../services/settings.service.js')
    const settingsService = appContainer.get(SettingsService)
    const claudeSettings = settingsService.getClaudeSettings()

    const agentLogger = logger.child({
      taskId: this.taskId,
      component: 'ClaudeCodeAgent',
    })

    agentLogger.debug('Building SDK options', {
      hasClaudeSettings: Boolean(claudeSettings),
      hasBinDir: Boolean(claudeSettings.binDir),
      hasConfigDir: Boolean(claudeSettings.configDir),
    })

    // Set environment variables for Claude Code if provided by settings
    if (claudeSettings.binDir && claudeSettings.binDir.trim() !== '') {
      process.env['CLAUDE_BIN_DIR'] = claudeSettings.binDir
      agentLogger.debug('Set CLAUDE_BIN_DIR environment variable', {
        binDir: claudeSettings.binDir,
      })
    }
    if (claudeSettings.configDir && claudeSettings.configDir.trim() !== '') {
      process.env['CLAUDE_CONFIG_DIR'] = claudeSettings.configDir
      agentLogger.debug('Set CLAUDE_CONFIG_DIR environment variable', {
        configDir: claudeSettings.configDir,
      })
    }

    // Translate agentMode to permissionMode
    const permissionMode = runOptions.agentMode === 'plan' ? 'plan' : 'bypassPermissions' // TODO: Should be 'acceptEdits'

    // Get upload directory and screenshots directory to add to additional directories
    const dataDir = getDataDir()
    const uploadDir = process.env['UPLOAD_DIR'] || path.join(dataDir, 'uploads')
    const screenshotsDir = path.join(dataDir, 'screenshots')

    // Configure allowed tools - base tools + HTTP MCP tools
    // HTTP MCP tools use the format: mcp__{serverName}__{toolName}
    const allowedTools = ['WebFetch', 'WebSearch', 'Bash']

    // Add HTTP MCP tool names (with mcp__ prefix and server name)
    allowedTools.push(
      'mcp__moatless-tools__updateTaskMetadata',
      'mcp__moatless-tools__install',
      'mcp__moatless-tools__updateRepositoryConfig',
      'mcp__moatless-tools__getRepositoryConfig',
    )

    // Add preview server tools if available
    if (runOptions.hasPreviewServer) {
      allowedTools.push(
        'mcp__moatless-tools__startOrRestartPreviewServer',
        'mcp__moatless-tools__getPreviewServerLogs',
        'mcp__moatless-tools__takePreviewScreenshot',
      )
    }

    agentLogger.info('Claude Code Agent: Preview server tools configuration', {
      taskId: this.taskId,
      hasPreviewServer: runOptions.hasPreviewServer,
      totalAllowedTools: allowedTools.length,
      previewToolsAdded: runOptions.hasPreviewServer ? 3 : 0,
      previewTools: runOptions.hasPreviewServer
        ? [
            'mcp__moatless-tools__startOrRestartPreviewServer',
            'mcp__moatless-tools__getPreviewServerLogs',
            'mcp__moatless-tools__takePreviewScreenshot',
          ]
        : [],
    })

    const sdkOptions: ClaudeCodeOptions = {
      allowedTools,
      permissionMode,
      ...options,
      cwd: runOptions.workspaceDir || options.cwd || process.cwd(),
      additionalDirectories: [uploadDir, screenshotsDir],
      stderr: (data: string) => {
        this.stderrOutput.push(data)
        agentLogger.debug('Claude Code stderr output', { stderr: data })
      },
    }

    // Add model if provided (prioritize runOptions.model over options.model)
    if (runOptions.model) {
      sdkOptions.model = runOptions.model
    } else if (options.model) {
      sdkOptions.model = options.model
    }

    if (runOptions.resume) {
      sdkOptions.resume = runOptions.resume
    }

    // Configure HTTP MCP server endpoint
    const mcpUrl =
      process.env['MCP_SERVER_URL'] || `http://localhost:${process.env['PORT'] || 8000}`

    // Start with moatless-tools HTTP MCP server
    const moatlessToolsServer: Record<string, McpServerConfig> = {
      'moatless-tools': {
        type: 'http' as const,
        url: `${mcpUrl}/api/v1/tasks/${this.taskId}/mcp`,
        headers: {},
      },
    }
    sdkOptions.mcpServers = moatlessToolsServer

    // Merge in user-configured MCP servers from settings
    const mcpSettings = await settingsService.getMcpSettings()
    if (mcpSettings.mcpServers && Object.keys(mcpSettings.mcpServers).length > 0) {
      sdkOptions.mcpServers = {
        ...moatlessToolsServer,
        ...mcpSettings.mcpServers,
      }

      // Add wildcard tool permissions for each user-configured MCP server
      // This allows all tools from user servers without requiring explicit permission
      for (const serverName of Object.keys(mcpSettings.mcpServers)) {
        allowedTools.push(`mcp__${serverName}__*`)
      }

      agentLogger.info('Added user-configured MCP servers', {
        userServerCount: Object.keys(mcpSettings.mcpServers).length,
        userServerNames: Object.keys(mcpSettings.mcpServers),
        allowedToolPatterns: allowedTools.filter((t) => t.includes('*')),
      })
    }

    agentLogger.info('MCP servers configured', {
      mcpUrl,
      taskId: this.taskId,
      totalServerCount: sdkOptions.mcpServers ? Object.keys(sdkOptions.mcpServers).length : 0,
      serverNames: sdkOptions.mcpServers ? Object.keys(sdkOptions.mcpServers) : [],
    })

    // Use the system prompt passed from TaskService
    if (runOptions.appendSystemPrompt) {
      sdkOptions.systemPrompt = {
        type: 'preset',
        preset: 'claude_code',
        append: runOptions.appendSystemPrompt,
      }
    }

    return sdkOptions
  }

  private getSDKMessageLogInfo(sdkMessage: ClaudeCodeSDKMessage): string {
    let detailedInfo = ''

    // Check if this message has content (assistant/user messages have message.content)
    if (
      'message' in sdkMessage &&
      sdkMessage.message &&
      'content' in sdkMessage.message &&
      sdkMessage.message.content
    ) {
      const content = Array.isArray(sdkMessage.message.content)
        ? sdkMessage.message.content
        : [sdkMessage.message.content]
      const contentTypes = content
        .map((item: unknown) => {
          if (!(item && typeof item === 'object' && 'type' in item)) {
            return 'unknown'
          }

          if (item.type === 'tool_use' && 'name' in item) {
            return `tool_use:${item.name}`
          }

          return item.type as string
        })
        .join(', ')
      detailedInfo += `, content: [${contentTypes}]`
    }

    // Add result info if it's a result message
    if (sdkMessage.type === 'result') {
      const subtype = 'subtype' in sdkMessage ? sdkMessage.subtype : 'unknown'
      const isError = 'is_error' in sdkMessage ? sdkMessage.is_error : false
      detailedInfo += `, subtype: ${subtype}, error: ${isError}`
    }

    return detailedInfo
  }

  /**
   * Parse stderr output to extract meaningful error messages
   * Filters out SDK debug information and returns the actual error
   */
  private parseStderrError(): string | null {
    if (this.stderrOutput.length === 0) {
      return null
    }

    // Filter out SDK debug lines that start with "Spawning Claude Code process:"
    const errorLines = this.stderrOutput.filter(
      (line) => !line.startsWith('Spawning Claude Code process:'),
    )

    if (errorLines.length === 0) {
      return null
    }

    // Return last 10 lines which typically contain the actual error
    const relevantLines = errorLines.slice(-10).join('\n').trim()
    return relevantLines || null
  }

  /**
   * Create and yield a message event with stderr error content
   */
  private *yieldStderrMessageEvent(
    agentLogger: ReturnType<typeof logger.child>,
  ): Generator<AgentEvent> {
    const stderrError = this.parseStderrError()
    if (!stderrError) {
      return
    }

    agentLogger.info('Emitting stderr as message event', {
      stderrLength: stderrError.length,
    })

    yield this.createEventWithContextTokens(
      {
        type: 'agent',
        subType: 'message',
        message: {
          type: 'message',
          role: 'assistant',
          content: [
            {
              type: 'text',
              text: `❌ **Agent Error**\n\n\`\`\`\n${stderrError}\n\`\`\``,
            },
          ],
        },
      },
      'agent',
      false,
      'message',
    )
  }

  /**
   * Log detailed error information for diagnostics
   */
  private logDetailedErrorInfo(
    agentLogger: ReturnType<typeof logger.child>,
    error: Error,
    exitCode?: number | null,
  ): void {
    const logContext = {
      stopRequested: this.stopRequested,
      sessionId: this.sessionId,
      lastSdkMessage: this.lastSdkMessage
        ? {
            type: this.lastSdkMessage.type,
            subtype: 'subtype' in this.lastSdkMessage ? this.lastSdkMessage.subtype : undefined,
            hasMessage: 'message' in this.lastSdkMessage,
          }
        : undefined,
      errorStack: error.stack,
      errorName: error.name,
      ...(exitCode !== undefined && { exitCode }),
      ...(this.stderrOutput.length > 0 && {
        stderrOutput: this.stderrOutput.join('\n'),
        stderrLineCount: this.stderrOutput.length,
      }),
    }

    if (exitCode !== undefined) {
      agentLogger.error('Unexpected Claude Code process exit', error, logContext)
    } else {
      agentLogger.error('Error in SDK query loop', error, logContext)
    }
  }

  /**
   * Create SDK message format - either simple string or AsyncIterable with content blocks
   * If images are provided, creates an async generator that yields a single SDKUserMessage
   * with content blocks (text + images). Otherwise, returns the simple string format.
   */
  // biome-ignore lint/suspicious/useAwait: Generator yields synchronously but must be async for SDK compatibility
  private async *createSDKMessageStream(
    message: string,
    images: ImageContentBlock[] | undefined,
    sessionId: string,
  ): AsyncGenerator<SDKUserMessage> {
    // If no images, the caller should use the simple string format instead
    if (!images || images.length === 0) {
      throw new Error('createSDKMessageStream should only be called when images are present')
    }

    // Create content blocks array with text first, then images
    const contentBlocks: MessageContentBlock[] = [{ type: 'text', text: message }]

    // Add image content blocks
    for (const image of images) {
      contentBlocks.push(image)
    }

    // Create the SDKUserMessage with content blocks
    // Use type assertion since we're creating a structure that matches the SDK's expected format
    // but we can't directly import the APIUserMessage type from @anthropic-ai/sdk
    const sdkMessage: SDKUserMessage = {
      type: 'user',
      message: {
        role: 'user',
        content: contentBlocks,
        // biome-ignore lint/suspicious/noExplicitAny: APIUserMessage type from @anthropic-ai/sdk not directly importable
      } as any,
      parent_tool_use_id: null,
      uuid: randomUUID() as `${string}-${string}-${string}-${string}-${string}`,
      session_id: sessionId,
    }

    // Yield the single message
    yield sdkMessage
  }

  // biome-ignore lint/suspicious/useAwait: cleanup is awaited elsewhere for consistency
  private async cleanup(): Promise<void> {
    if (this.abortController && !this.abortController.signal.aborted) {
      this.abortController.abort()
    }
    this.abortController = undefined
  }

  async sendMessage(message: string, runOptions: AgentRunOptions = {}): Promise<string> {
    const agentLogger = logger.child({
      taskId: this.taskId,
      component: 'ClaudeCodeAgent.sendMessage',
    })

    try {
      const sdkOptions = await this.buildSDKOptions(runOptions)
      const abortController = new AbortController()

      let responseText = ''

      // Use the SDK query function
      for await (const sdkMessage of query({
        prompt: message,
        options: {
          ...sdkOptions,
          abortController,
        },
      })) {
        agentLogger.debug('Received SDK message', { sdkMessage })

        if (sdkMessage.type === 'result' && 'result' in sdkMessage && sdkMessage.result) {
          // For result messages, use the result field
          responseText += sdkMessage.result
        } else if (
          (sdkMessage.type === 'assistant' ||
            sdkMessage.type === 'user' ||
            sdkMessage.type === 'system') &&
          'content' in sdkMessage &&
          sdkMessage.content
        ) {
          // For message types, use the content field
          responseText += sdkMessage.content
        }
      }

      return responseText
    } catch (error) {
      agentLogger.error('Error in sendMessage', error as Error)
      throw error
    }
  }

  async getBranchNameAndSummary(
    message: string,
    runOptions: AgentRunOptions = {},
  ): Promise<BranchNameAndSummary | null> {
    const agentLogger = logger.child({
      taskId: this.taskId,
      component: 'ClaudeCodeAgent.getBranchNameAndSummary',
    })

    try {
      // Build SDK options with specific configuration for branch name generation
      const sdkOptions = await this.buildSDKOptions(runOptions)

      // Override options for this specific task
      const branchNameOptions: ClaudeCodeOptions = {
        ...sdkOptions,
        maxTurns: 1,
        mcpServers: {}, // No tools for branch name generation
        model: sdkOptions.model || 'claude-sonnet-4-5-20250929',
      }

      // Create the formatted prompt with the user's request
      const formattedPrompt = createBranchNamePrompt(message)

      const abortController = new AbortController()
      let responseText = ''

      // Use the SDK query function with branch name specific options
      for await (const sdkMessage of query({
        prompt: formattedPrompt,
        options: {
          ...branchNameOptions,
          abortController,
        },
      })) {
        agentLogger.debug('Received SDK message for branch name', {
          sdkMessage,
        })

        if (sdkMessage.type === 'result' && 'result' in sdkMessage && sdkMessage.result) {
          // For result messages, use the result field
          responseText += sdkMessage.result
        } else if (
          (sdkMessage.type === 'assistant' ||
            sdkMessage.type === 'user' ||
            sdkMessage.type === 'system') &&
          'content' in sdkMessage &&
          sdkMessage.content
        ) {
          // For message types, use the content field
          responseText += sdkMessage.content
        }
      }

      agentLogger.debug('Raw response for branch name', {
        response: responseText,
      })

      const parseResult = parseBranchNameAndSummary(responseText)

      if (parseResult.found && parseResult.data) {
        agentLogger.debug('Successfully parsed branch name and summary', parseResult.data)
        return parseResult.data
      }
      agentLogger.warn('Failed to parse branch name and summary', {
        error: parseResult.error,
        response: responseText,
      })
      return null
    } catch (error) {
      agentLogger.error('Error in getBranchNameAndSummary', error as Error)
      throw error
    }
  }
}
