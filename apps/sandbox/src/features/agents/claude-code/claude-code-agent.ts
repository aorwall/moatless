import { randomUUID } from 'node:crypto'
import path from 'node:path'
import { query, type SDKUserMessage } from '@anthropic-ai/claude-agent-sdk'
import { SANDBOX_DATA_DIR, SANDBOX_PLUGINS_DIR } from '@moatless/sandbox-schemas'
import type {
  AgentEvent,
  AskUserQuestionResponse,
  ClaudeCodeOptions,
  ClaudeCodeSDKMessage,
  ExitPlanModeResponse,
} from '@moatless/types'
import type { Logger } from 'pino'
import { getAgentConfig } from '../../../config/environment'
import { logger } from '../../../shared/logger'
import {
  type AgentEventCallback,
  type AgentOptions,
  type AgentRunRequest,
  BaseAgent,
} from '../base/base-agent'
import { calculateContextTokens } from './context-tokens'

// ===== User Input Tools Support =====
// Module-scoped state for pending user input (only one at a time)
type PendingUserInput = {
  toolName: 'AskUserQuestion' | 'ExitPlanMode'
  resolve: (response: AskUserQuestionResponse | ExitPlanModeResponse) => void
  reject: (error: Error) => void
}

let pendingUserInput: PendingUserInput | null = null

/**
 * Resolve the pending user input with the provided response.
 * Called by the agent routes when user submits their answer.
 */
export function resolveUserInput(
  response: AskUserQuestionResponse | ExitPlanModeResponse,
): boolean {
  if (pendingUserInput) {
    pendingUserInput.resolve(response)
    pendingUserInput = null
    return true
  }
  return false
}

/**
 * Check if there is a pending user input request
 */
export function hasPendingUserInput(): boolean {
  return pendingUserInput !== null
}

/**
 * Get the tool name of the pending user input request
 */
export function getPendingToolName(): 'AskUserQuestion' | 'ExitPlanMode' | null {
  return pendingUserInput?.toolName ?? null
}

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

  async run(
    request: AgentRunRequest,
    workspaceDir: string,
    onEvent?: AgentEventCallback,
  ): Promise<void> {
    this.stopRequested = false
    this.abortController = new AbortController()
    this.onEvent = onEvent
    this.stderrOutput = []

    const agentLogger = this.createLogger(request)
    this.logRunStart(agentLogger, request, workspaceDir)

    // Extract message from content
    const message = this.extractMessage(request)

    // Track whether we've emitted 'started' event (for 'stopped' event emission)
    let hasEmittedStarted = false

    try {
      const sdkOptions = await this.buildSDKOptions(request, workspaceDir)
      this.logSDKOptions(agentLogger, sdkOptions, request)

      let eventCount = 0
      let hasStartedProcessing = false
      let hasEmittedCompletion = false

      // Determine prompt format based on presence of non-text content blocks (images, etc.)
      const nonTextContentBlocks = request.content.filter((c) => c.type !== 'text')
      const hasContentBlocks = nonTextContentBlocks.length > 0
      const promptFormat = hasContentBlocks ? 'content blocks' : 'simple string'

      agentLogger.debug(
        {
          promptLength: message.length,
          promptFormat,
          contentBlockCount: nonTextContentBlocks.length,
          taskId: this.taskId,
        },
        'Initiating Claude Code SDK query',
      )

      // Create prompt - use AsyncIterable format if content blocks are present
      const prompt = hasContentBlocks
        ? this.createSDKMessageStream(message, nonTextContentBlocks, randomUUID())
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
          this.emitEvent(
            this.createEventWithContextTokens(
              { type: 'agent', subType: 'started' },
              'agent',
              false,
            ),
          )
        }

        if (this.stopRequested) {
          agentLogger.info({ eventCount }, 'Stop requested, breaking from SDK query loop')
          break
        }

        this.captureSessionId(agentLogger, sdkMessage)
        this.extractPlanFromMessage(agentLogger, sdkMessage)

        if (sdkMessage.type !== 'system' || hasStartedProcessing) {
          hasStartedProcessing = true
        }

        if (sdkMessage.type === 'result') {
          hasEmittedCompletion = true
          this.handleResultMessage(agentLogger, sdkMessage)
        } else {
          this.emitEvent(this.createEventWithContextTokens(sdkMessage, 'agent', false, 'message'))
        }
      }

      // Emit 'stopped' event if agent was stopped without receiving a completion event
      if (this.stopRequested && !hasEmittedCompletion && hasEmittedStarted) {
        agentLogger.info('Emitting stopped event for user-initiated stop')
        this.emitEvent(
          this.createEventWithContextTokens({ type: 'agent', subType: 'stopped' }, 'agent', false),
        )
      }

      this.logQueryCompletion(agentLogger, eventCount, hasStartedProcessing)
    } catch (error) {
      this.handleError(agentLogger, error, hasEmittedStarted)
    } finally {
      agentLogger.debug('Cleaning up Claude Code agent')
      await this.cleanup()
    }
  }

  /**
   * Run the agent with multiple batched requests.
   *
   * This method processes multiple queued requests as a single SDK call by
   * yielding an SDKUserMessage for each request in sequence. The SDK processes
   * all messages within the same session.
   *
   * @param requests - Array of agent run requests to batch
   * @param workspaceDir - Workspace directory path
   * @param onEvent - Optional callback for receiving agent events
   */
  async runBatched(
    requests: AgentRunRequest[],
    workspaceDir: string,
    onEvent?: AgentEventCallback,
  ): Promise<void> {
    if (requests.length === 0) {
      throw new Error('Cannot run batched with empty requests array')
    }

    // For single request, delegate to regular run
    if (requests.length === 1) {
      return this.run(requests[0], workspaceDir, onEvent)
    }

    this.stopRequested = false
    this.abortController = new AbortController()
    this.onEvent = onEvent
    this.stderrOutput = []

    // Use the last request for logging and options (most recent user intent)
    const lastRequest = requests[requests.length - 1]
    const agentLogger = this.createLogger(lastRequest)

    agentLogger.info(
      {
        taskId: this.taskId,
        batchSize: requests.length,
        hasResumeSession: Boolean(lastRequest.resume),
        workspaceDir,
      },
      'Starting batched Claude Code agent execution',
    )

    // Track whether we've emitted 'started' event (for 'stopped' event emission)
    let hasEmittedStarted = false

    try {
      const sdkOptions = await this.buildSDKOptions(lastRequest, workspaceDir)
      this.logSDKOptions(agentLogger, sdkOptions, lastRequest)

      let eventCount = 0
      let hasStartedProcessing = false
      let hasEmittedCompletion = false

      // Generate session ID for batched messages
      const batchSessionId = this.sessionId || randomUUID()

      agentLogger.debug(
        {
          batchSize: requests.length,
          batchSessionId,
          taskId: this.taskId,
        },
        'Initiating batched Claude Code SDK query',
      )

      // Create the batched message stream
      const prompt = this.createBatchedSDKMessageStream(requests, batchSessionId)

      for await (const sdkMessage of query({
        prompt,
        options: { ...sdkOptions, abortController: this.abortController },
      })) {
        eventCount++
        this.lastSdkMessage = sdkMessage
        this.logSDKMessage(agentLogger, sdkMessage, eventCount)

        if (!hasEmittedStarted) {
          hasEmittedStarted = true
          this.emitEvent(
            this.createEventWithContextTokens(
              { type: 'agent', subType: 'started' },
              'agent',
              false,
            ),
          )
        }

        if (this.stopRequested) {
          agentLogger.info({ eventCount }, 'Stop requested, breaking from batched SDK query loop')
          break
        }

        this.captureSessionId(agentLogger, sdkMessage)
        this.extractPlanFromMessage(agentLogger, sdkMessage)

        if (sdkMessage.type !== 'system' || hasStartedProcessing) {
          hasStartedProcessing = true
        }

        if (sdkMessage.type === 'result') {
          hasEmittedCompletion = true
          this.handleResultMessage(agentLogger, sdkMessage)
        } else {
          this.emitEvent(this.createEventWithContextTokens(sdkMessage, 'agent', false, 'message'))
        }
      }

      // Emit 'stopped' event if agent was stopped without receiving a completion event
      if (this.stopRequested && !hasEmittedCompletion && hasEmittedStarted) {
        agentLogger.info('Emitting stopped event for user-initiated stop (batched)')
        this.emitEvent(
          this.createEventWithContextTokens({ type: 'agent', subType: 'stopped' }, 'agent', false),
        )
      }

      this.logQueryCompletion(agentLogger, eventCount, hasStartedProcessing)
    } catch (error) {
      this.handleError(agentLogger, error, hasEmittedStarted)
    } finally {
      agentLogger.debug('Cleaning up batched Claude Code agent')
      await this.cleanup()
    }
  }

  /**
   * Create an async generator that yields SDKUserMessage for each request in the batch.
   *
   * Each message shares the same session_id, allowing Claude to process them
   * sequentially within a single SDK query call.
   */
  // biome-ignore lint/suspicious/useAwait: Generator yields synchronously but must be async for SDK compatibility
  private async *createBatchedSDKMessageStream(
    requests: AgentRunRequest[],
    sessionId: string,
  ): AsyncGenerator<SDKUserMessage> {
    for (const request of requests) {
      // Extract text message
      const message = this.extractMessage(request)

      // Get non-text content blocks (images, etc.)
      const nonTextContentBlocks = request.content.filter((c) => c.type !== 'text')

      // Create content blocks array
      const messageBlocks: MessageContentBlock[] = [{ type: 'text', text: message }]

      // Add any non-text content blocks
      for (const block of nonTextContentBlocks) {
        messageBlocks.push(block as MessageContentBlock)
      }

      // Create the SDKUserMessage
      const sdkMessage: SDKUserMessage = {
        type: 'user',
        message: {
          role: 'user',
          content: messageBlocks,
          // biome-ignore lint/suspicious/noExplicitAny: APIUserMessage type from @anthropic-ai/sdk not directly importable
        } as any,
        parent_tool_use_id: null,
        uuid: randomUUID() as `${string}-${string}-${string}-${string}-${string}`,
        session_id: sessionId,
      }

      yield sdkMessage
    }
  }

  async stop(): Promise<void> {
    this.stopRequested = true

    if (this.abortController && !this.abortController.signal.aborted) {
      this.abortController.abort()
    }

    await this.cleanup()
  }

  private createLogger(request: AgentRunRequest) {
    return logger.child({
      taskId: this.taskId,
      component: 'ClaudeCodeAgent',
    })
  }

  private logRunStart(agentLogger: Logger, request: AgentRunRequest, workspaceDir: string): void {
    agentLogger.debug(
      {
        taskId: this.taskId,
        hasResumeSession: Boolean(request.resume),
        workspaceDir,
      },
      'Starting Claude Code agent execution',
    )
  }

  private logSDKOptions(
    agentLogger: Logger,
    sdkOptions: ClaudeCodeOptions,
    request: AgentRunRequest,
  ): void {
    // Get agent config for logging (plugins/systemPrompt come from config, not request)
    const sandboxAgentConfig = getAgentConfig()

    agentLogger.info(
      {
        model: sdkOptions.model,
        allowedTools: sdkOptions.allowedTools,
        permissionMode: sdkOptions.permissionMode,
        agentMode: request.agentMode,
        cwd: sdkOptions.cwd,
        resume: sdkOptions.resume,
        systemPromptLength: sandboxAgentConfig.systemPrompt?.length || 0,
        additionalDirectories: sdkOptions.additionalDirectories,
        settingSources: sdkOptions.settingSources,
        pathToClaudeCodeExecutable: sdkOptions.pathToClaudeCodeExecutable,
        pluginCount: sdkOptions.plugins?.length || 0,
        pluginPaths: sdkOptions.plugins?.map((p) => p.path) || [],
        hasSystemPrompt: !!sdkOptions.systemPrompt,
      },
      'Claude Code SDK options configured',
    )
  }

  private logSDKMessage(
    agentLogger: Logger,
    sdkMessage: ClaudeCodeSDKMessage,
    eventCount: number,
  ): void {
    agentLogger.debug({ eventCount, sdkMessage }, 'Received SDK message')
    const logInfo = this.getSDKMessageLogInfo(sdkMessage)
    agentLogger.info({ eventCount }, `SDK Event: type=${sdkMessage.type}${logInfo}`)

    // Check for error content items and log warnings
    this.logErrorContentItems(agentLogger, sdkMessage, eventCount)
  }

  private logErrorContentItems(
    agentLogger: Logger,
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

        agentLogger.warn(
          {
            eventCount,
            contentType,
            errorContent,
            toolUseId,
            fullContentItem: contentItem,
          },
          `SDK message contains error content: ${contentType} error`,
        )
      }
    }
  }

  private captureSessionId(agentLogger: Logger, sdkMessage: ClaudeCodeSDKMessage): void {
    if (!this.sessionId && sdkMessage.session_id) {
      this.sessionId = sdkMessage.session_id
      agentLogger.info({ sessionId: this.sessionId }, 'Session ID captured from SDK')
    }
  }

  private extractPlanFromMessage(agentLogger: Logger, sdkMessage: ClaudeCodeSDKMessage): void {
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
          agentLogger.info(
            {
              planLength: this.extractedPlan?.length,
            },
            'ExitPlanMode detected, plan extracted',
          )
          break
        }
      }
    }
  }

  private handleResultMessage(agentLogger: Logger, sdkMessage: ClaudeCodeSDKMessage): void {
    const isError = 'is_error' in sdkMessage ? sdkMessage.is_error : false
    const subtype = 'subtype' in sdkMessage ? sdkMessage.subtype : undefined
    const hasResult = 'result' in sdkMessage && sdkMessage.result !== null

    // Extract errors array from SDK message
    const errors =
      'errors' in sdkMessage && Array.isArray(sdkMessage.errors)
        ? (sdkMessage.errors as string[])
        : []

    agentLogger.info(
      {
        isError,
        subtype,
        hasExtractedPlan: Boolean(this.extractedPlan),
        hasResult,
        errors,
        totalCostUsd: 'total_cost_usd' in sdkMessage ? sdkMessage.total_cost_usd : undefined,
      },
      'Received result message from SDK',
    )

    // Log detailed information for error result subtypes
    if (subtype === 'error_max_turns' || subtype === 'error_during_execution') {
      agentLogger.error(
        {
          errorSubtype: subtype,
          errors,
          numTurns: 'num_turns' in sdkMessage ? sdkMessage.num_turns : undefined,
          durationMs: 'duration_ms' in sdkMessage ? sdkMessage.duration_ms : undefined,
          hasStderrOutput: this.stderrOutput.length > 0,
          stderrLineCount: this.stderrOutput.length,
          stderrPreview:
            this.stderrOutput.length > 0 ? this.stderrOutput.slice(0, 3).join('\n') : undefined,
        },
        errors.length > 0 ? `SDK error: ${errors[0]}` : 'SDK returned error result',
      )
    }

    this.emitEvent(this.createResultEvent(agentLogger, sdkMessage, isError))
  }

  private createResultEvent(
    agentLogger: Logger,
    sdkMessage: ClaudeCodeSDKMessage,
    isError: boolean,
  ): AgentEvent {
    if (this.extractedPlan) {
      agentLogger.info(
        {
          planLength: this.extractedPlan.length,
        },
        'Including extracted plan in result event',
      )
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
    agentLogger: Logger,
    eventCount: number,
    hasStartedProcessing: boolean,
  ): void {
    agentLogger.info(
      {
        totalEvents: eventCount,
        hasStartedProcessing,
        stopRequested: this.stopRequested,
        sessionId: this.sessionId,
      },
      'SDK query loop completed',
    )

    if (eventCount === 1 && !hasStartedProcessing) {
      const errorMessage =
        'SDK query loop exited after only init message - this indicates an authentication or configuration issue. ' +
        'Check: 1) OAuth token is valid, 2) Claude executable exists and is correct, ' +
        `3) stderr output: ${this.stderrOutput.join('\n') || 'none'}`

      agentLogger.error({ eventCount, stderrOutput: this.stderrOutput }, errorMessage)

      // Throw to trigger error handling instead of silent completion
      throw new Error(errorMessage)
    }
  }

  private handleError(agentLogger: Logger, error: unknown, hasEmittedStarted = false): void {
    const errorMessage = error instanceof Error ? error.message : String(error)
    const errorName = error instanceof Error ? error.name : 'Unknown'

    // When stop is requested, handle ALL errors gracefully (don't throw)
    // This ensures user-initiated stops don't produce error logs
    if (this.stopRequested) {
      const isAbortError =
        errorName === 'AbortError' ||
        errorMessage.includes('abort') ||
        errorMessage.includes('aborted')

      if (isAbortError) {
        agentLogger.info(
          {
            sessionId: this.sessionId,
            errorMessage,
          },
          'Agent stopped gracefully by user request',
        )
      } else {
        // Non-abort error during stop request - log as warning since stop was requested
        agentLogger.warn(
          {
            sessionId: this.sessionId,
            errorMessage,
            errorName,
          },
          'Agent stopped with unexpected error type (stop was requested)',
        )
      }

      // Emit 'stopped' event so backend knows agent has stopped
      if (hasEmittedStarted) {
        agentLogger.info('Emitting stopped event from error handler')
        this.emitEvent(
          this.createEventWithContextTokens({ type: 'agent', subType: 'stopped' }, 'agent', false),
        )
      }

      return // Don't throw - stop was requested
    }

    // Stop was NOT requested - this is an actual error
    if (error instanceof Error && error.message.includes('Claude Code process exited')) {
      const exitCodeMatch = error.message.match(/code (\d+)/)
      const exitCode = exitCodeMatch?.[1] ? Number.parseInt(exitCodeMatch[1], 10) : null
      const isExpectedExitCode = exitCode === 143 || exitCode === 130

      // Exit codes 143 (SIGTERM) and 130 (SIGINT) are expected termination signals
      if (isExpectedExitCode) {
        agentLogger.info(
          {
            exitCode,
            sessionId: this.sessionId,
          },
          'Claude Code process terminated by signal',
        )
        return
      }

      // Log detailed error information for diagnostics
      this.logDetailedErrorInfo(agentLogger, error, exitCode)

      // Emit stderr as a message event if we have error output
      this.emitStderrMessageEvent(agentLogger)

      this.emitEvent(
        this.createEventWithContextTokens(
          {
            type: 'agent',
            subType: 'error',
            error: error.message,
            exitCode,
          },
          'agent',
          true,
        ),
      )

      throw error
    }

    this.logDetailedErrorInfo(
      agentLogger,
      error instanceof Error ? error : new Error(String(error)),
      undefined,
    )

    // Emit stderr as a message event if we have error output
    this.emitStderrMessageEvent(agentLogger)

    this.emitEvent(
      this.createEventWithContextTokens(
        {
          type: 'agent',
          subType: 'error',
          error: error instanceof Error ? error.message : String(error),
          stackTrace: error instanceof Error ? error.stack : undefined,
        },
        'agent',
        true,
      ),
    )

    throw error
  }

  getAgentType(): string {
    return 'claude-code'
  }

  /**
   * Get the current session ID if available.
   * Used by AgentRequestConsumer to track sessions for resume functionality.
   */
  getSessionId(): string | undefined {
    return this.sessionId
  }

  /**
   * Extract text message from content array.
   * Combines all text content blocks into a single message.
   */
  private extractMessage(request: AgentRunRequest): string {
    if (!request.content || request.content.length === 0) {
      return ''
    }
    return request.content
      .filter((c): c is { type: 'text'; text: string } => c.type === 'text')
      .map((c) => c.text)
      .join('\n')
  }

  /**
   * Resolve plugin identifiers to sandbox-absolute paths.
   * Plugin ID format: {plugin-name}@{marketplace-name}
   * Resolves to: /opt/moatless/.claude/plugins/marketplaces/{marketplace-name}/{plugin-name}
   */
  private resolvePluginPaths(pluginIds: string[], agentLogger: Logger): string[] {
    return pluginIds
      .map((pluginId) => {
        const atIndex = pluginId.lastIndexOf('@')
        if (atIndex === -1) {
          agentLogger.warn({ pluginId }, 'Invalid plugin ID format (missing @)')
          return null
        }
        const pluginName = pluginId.substring(0, atIndex)
        const marketplaceName = pluginId.substring(atIndex + 1)
        return path.join(SANDBOX_PLUGINS_DIR, 'marketplaces', marketplaceName, pluginName)
      })
      .filter((p): p is string => p !== null)
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
  private async buildSDKOptions(
    request: AgentRunRequest,
    workspaceDir: string,
  ): Promise<ClaudeCodeOptions> {
    const options = this.options as ClaudeCodeOptions

    // Settings come from environment variables
    // The main API sets these env vars when creating the pod
    const claudeSettings = {
      configDir: process.env['CLAUDE_CONFIG_DIR'],
      binDir: process.env['CLAUDE_BIN_DIR'],
      oauthToken: process.env['CLAUDE_CODE_OAUTH_TOKEN'],
    }

    const agentLogger = logger.child({
      taskId: this.taskId,
      component: 'ClaudeCodeAgent',
    })

    agentLogger.debug(
      {
        hasClaudeSettings: Boolean(claudeSettings),
        hasBinDir: Boolean(claudeSettings.binDir),
        hasConfigDir: Boolean(claudeSettings.configDir),
      },
      'Building SDK options',
    )

    // Set environment variables for Claude Code if provided by settings
    if (claudeSettings.binDir && claudeSettings.binDir.trim() !== '') {
      process.env['CLAUDE_BIN_DIR'] = claudeSettings.binDir
      agentLogger.debug(
        {
          binDir: claudeSettings.binDir,
        },
        'Set CLAUDE_BIN_DIR environment variable',
      )
    }
    if (claudeSettings.configDir && claudeSettings.configDir.trim() !== '') {
      process.env['CLAUDE_CONFIG_DIR'] = claudeSettings.configDir
      agentLogger.debug(
        {
          configDir: claudeSettings.configDir,
        },
        'Set CLAUDE_CONFIG_DIR environment variable',
      )
    }

    // Get Claude token from options (provided by AgentController) or fall back to env
    const providedToken = (this.options as AgentOptions).apiKey
    if (providedToken && providedToken.trim() !== '') {
      process.env['CLAUDE_CODE_OAUTH_TOKEN'] = providedToken
      agentLogger.info('Set CLAUDE_CODE_OAUTH_TOKEN from database token (OAuth/PAT)')
    } else if (claudeSettings.oauthToken && claudeSettings.oauthToken.trim() !== '') {
      process.env['CLAUDE_CODE_OAUTH_TOKEN'] = claudeSettings.oauthToken
      agentLogger.info('Set CLAUDE_CODE_OAUTH_TOKEN from environment variable')
    } else {
      agentLogger.warn('No Claude token available')
    }

    // ADDED: Validate authentication before proceeding
    if (!(process.env['CLAUDE_CODE_OAUTH_TOKEN'] || process.env['ANTHROPIC_API_KEY'])) {
      throw new Error(
        'Claude Code authentication not configured: Neither CLAUDE_CODE_OAUTH_TOKEN nor ANTHROPIC_API_KEY is set',
      )
    }

    agentLogger.debug(
      {
        hasOAuthToken: !!process.env['CLAUDE_CODE_OAUTH_TOKEN'],
        hasApiKey: !!process.env['ANTHROPIC_API_KEY'],
      },
      'Claude authentication configured',
    )

    // Plan mode behavior is controlled by system prompt, not SDK permissions
    // SDK 'plan' permission mode forces ExitPlanMode for every action, causing loops
    const permissionMode = 'bypassPermissions' // Plan/edit mode handled via system prompt

    // Get upload directory from environment
    const dataDir = process.env['DATA_DIR'] || SANDBOX_DATA_DIR
    const uploadDir = process.env['UPLOAD_DIR'] || path.join(dataDir, 'uploads')

    // Ensure uploads directory exists before passing to agent
    const { mkdir } = await import('node:fs/promises')
    await mkdir(uploadDir, { recursive: true })

    // Configure allowed tools - base tools + HTTP MCP tools
    // HTTP MCP tools use the format: mcp__{serverName}__{toolName}
    // Include file operation tools for reading/writing/editing files
    const allowedTools = [
      'WebFetch',
      'WebSearch',
      'Bash',
      'Read',
      'Write',
      'Edit',
      'MultiEdit',
      'Glob',
      'Grep',
      'LS',
    ]

    // In sandbox mode, Claude Code is installed globally via npm
    // Set pathToClaudeCodeExecutable to the global install location
    const claudeExecutablePath = claudeSettings.binDir
      ? `${claudeSettings.binDir}/claude`
      : '/usr/bin/claude'

    // ADDED: Validate Claude executable exists
    const { existsSync } = await import('node:fs')
    if (!existsSync(claudeExecutablePath)) {
      throw new Error(
        `Claude executable not found at ${claudeExecutablePath}. ` +
          `CLAUDE_BIN_DIR=${claudeSettings.binDir || 'not set'}`,
      )
    }

    agentLogger.debug({ claudeExecutablePath }, 'Claude executable validated')

    const sdkOptions: ClaudeCodeOptions = {
      allowedTools,
      permissionMode,
      ...options,
      cwd: workspaceDir || options.cwd || process.cwd(),
      additionalDirectories: [uploadDir],
      settingSources: ['project', 'user'],
      pathToClaudeCodeExecutable: claudeExecutablePath,
      stderr: (data: string) => {
        this.stderrOutput.push(data)
        // Log at WARN level so it's visible even without DEBUG
        agentLogger.warn({ stderr: data }, 'Claude Code stderr output')
      },
      // Handle user input tools (AskUserQuestion, ExitPlanMode)
      canUseTool: async (toolName, toolInput) => {
        if (toolName === 'AskUserQuestion' || toolName === 'ExitPlanMode') {
          // Emit pending_response event for channel notification BEFORE waiting
          this.emitPendingResponseEvent(toolName, toolInput)
          return await this.handleUserInputTool(toolName, toolInput)
        }
        return { behavior: 'allow', updatedInput: toolInput }
      },
    }

    agentLogger.debug({ claudeExecutablePath }, 'Set Claude executable path')

    // Get agent config from REPO_CONFIG (static, per-task settings)
    // This includes: plugins, timeout, model (default), systemPrompt
    const sandboxAgentConfig = getAgentConfig()

    // Add model if provided (prioritize: request.model > agentConfig.model > options.model)
    if (request.model) {
      sdkOptions.model = request.model
    } else if (sandboxAgentConfig.model) {
      sdkOptions.model = sandboxAgentConfig.model
    } else if (options.model) {
      sdkOptions.model = options.model
    }

    // Only pass string session IDs to SDK (boolean true should have been resolved by consumer)
    if (typeof request.resume === 'string') {
      sdkOptions.resume = request.resume
    }

    // Configure plugins from REPO_CONFIG.agentConfig (static, per-task)
    // Plugin IDs are resolved to paths here in sandbox
    if (sandboxAgentConfig.plugins && sandboxAgentConfig.plugins.length > 0) {
      const resolvedPaths = this.resolvePluginPaths(sandboxAgentConfig.plugins, agentLogger)
      sdkOptions.plugins = resolvedPaths
        .filter((p) => existsSync(p))
        .map((p) => ({ type: 'local' as const, path: p }))

      agentLogger.debug(
        {
          configuredPlugins: sandboxAgentConfig.plugins.length,
          resolvedPaths: resolvedPaths.length,
          activePlugins: sdkOptions.plugins.length,
          pluginPaths: sdkOptions.plugins.map((p) => p.path),
        },
        'Plugins configured from REPO_CONFIG.agentConfig',
      )
    } else {
      agentLogger.debug('No plugins configured in REPO_CONFIG.agentConfig')
    }

    // Use the system prompt from REPO_CONFIG (static, per-task)
    if (sandboxAgentConfig.systemPrompt) {
      sdkOptions.systemPrompt = {
        type: 'preset',
        preset: 'claude_code',
        append: sandboxAgentConfig.systemPrompt,
      }
    }

    // Append per-request system prompt from backend (e.g., preview server instructions)
    if (request.appendSystemPrompt) {
      if (
        sdkOptions.systemPrompt &&
        typeof sdkOptions.systemPrompt === 'object' &&
        sdkOptions.systemPrompt.append
      ) {
        sdkOptions.systemPrompt.append += `\n\n${request.appendSystemPrompt}`
      } else {
        sdkOptions.systemPrompt = {
          type: 'preset',
          preset: 'claude_code',
          append: request.appendSystemPrompt,
        }
      }
      agentLogger.debug(
        { appendLength: request.appendSystemPrompt.length },
        'Per-request system prompt appended',
      )
    }

    return sdkOptions
  }

  /**
   * Emit a pending_response event to notify channels that the agent is waiting for user input.
   * This is emitted BEFORE the agent blocks waiting for response, allowing Slack/Email to prompt user.
   */
  private emitPendingResponseEvent(
    toolName: 'AskUserQuestion' | 'ExitPlanMode',
    toolInput: Record<string, unknown>,
  ): void {
    const agentLogger = logger.child({ taskId: this.taskId, toolName })
    agentLogger.info('Emitting pending_response event for interactive tool')

    this.emitEvent({
      eventId: crypto.randomUUID(),
      agentType: this.getAgentType(),
      type: 'agent',
      subType: 'pending_response',
      taskId: this.taskId,
      timestamp: new Date(),
      isError: false,
      payload: {
        toolName,
        toolInput,
        agentSessionId: this.sessionId,
      },
    } as AgentEvent)
  }

  /**
   * Handle user input tools (AskUserQuestion, ExitPlanMode)
   * Pauses execution until user provides input via the API endpoint
   */
  private async handleUserInputTool(
    toolName: 'AskUserQuestion' | 'ExitPlanMode',
    input: Record<string, unknown>,
  ): Promise<
    | { behavior: 'allow'; updatedInput: Record<string, unknown> }
    | { behavior: 'deny'; message: string }
  > {
    const agentLogger = logger.child({ taskId: this.taskId, toolName })

    agentLogger.info({ input }, `${toolName} tool invoked, waiting for user input`)

    // Create promise that will be resolved when response arrives
    // No timeout - wait indefinitely for user input
    const responsePromise = new Promise<AskUserQuestionResponse | ExitPlanModeResponse>(
      (resolve) => {
        // No reject needed - we wait indefinitely for user input
        pendingUserInput = { toolName, resolve, reject: () => undefined }
      },
    )

    // Wait for response (tool_use message already flows to frontend via SSE)
    const response = await responsePromise

    agentLogger.info({ toolName, response }, 'User input received')

    // Handle response based on tool type
    if (toolName === 'AskUserQuestion') {
      const answerResponse = response as AskUserQuestionResponse
      return {
        behavior: 'allow',
        updatedInput: {
          ...input,
          answers: answerResponse.answers,
        },
      }
    }

    if (toolName === 'ExitPlanMode') {
      const planResponse = response as ExitPlanModeResponse
      if (planResponse.approved) {
        return {
          behavior: 'allow',
          updatedInput: input,
        }
      }
      return {
        behavior: 'deny',
        message: planResponse.feedback || 'User rejected the plan',
      }
    }

    return { behavior: 'allow', updatedInput: input }
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

    // Filter out SDK debug lines that start with spawn info
    const errorLines = this.stderrOutput.filter(
      (line) =>
        !(
          line.startsWith('Spawning Claude Code process:') ||
          line.startsWith('Spawning Claude Code native binary:')
        ),
    )

    if (errorLines.length === 0) {
      return null
    }

    // Return last 10 lines which typically contain the actual error
    const relevantLines = errorLines.slice(-10).join('\n').trim()
    return relevantLines || null
  }

  /**
   * Emit a message event with stderr error content
   */
  private emitStderrMessageEvent(agentLogger: Logger): void {
    const stderrError = this.parseStderrError()
    if (!stderrError) {
      return
    }

    agentLogger.info(
      {
        stderrLength: stderrError.length,
      },
      'Emitting stderr as message event',
    )

    this.emitEvent(
      this.createEventWithContextTokens(
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
      ),
    )
  }

  /**
   * Log detailed error information for diagnostics
   */
  private logDetailedErrorInfo(agentLogger: Logger, error: Error, exitCode?: number | null): void {
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
      agentLogger.error({ err: error, ...logContext }, 'Unexpected Claude Code process exit')
    } else {
      agentLogger.error({ err: error, ...logContext }, 'Error in SDK query loop')
    }
  }

  /**
   * Create SDK message format - either simple string or AsyncIterable with content blocks
   * If content blocks are provided, creates an async generator that yields a single SDKUserMessage
   * with content blocks (text message + file content blocks). Otherwise, returns the simple string format.
   */
  // biome-ignore lint/suspicious/useAwait: Generator yields synchronously but must be async for SDK compatibility
  private async *createSDKMessageStream(
    message: string,
    contentBlocks: import('@moatless/types').UserContent[] | undefined,
    sessionId: string,
  ): AsyncGenerator<SDKUserMessage> {
    // If no content blocks, the caller should use the simple string format instead
    if (!contentBlocks || contentBlocks.length === 0) {
      throw new Error(
        'createSDKMessageStream should only be called when content blocks are present',
      )
    }

    // Create content blocks array with text first, then all file content blocks
    const messageBlocks: MessageContentBlock[] = [{ type: 'text', text: message }]

    // Add all content blocks (images, text files, etc.)
    for (const block of contentBlocks) {
      messageBlocks.push(block as MessageContentBlock)
    }

    // Create the SDKUserMessage with content blocks
    // Use type assertion since we're creating a structure that matches the SDK's expected format
    // but we can't directly import the APIUserMessage type from @anthropic-ai/sdk
    const sdkMessage: SDKUserMessage = {
      type: 'user',
      message: {
        role: 'user',
        content: messageBlocks,
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

  async sendMessage(request: AgentRunRequest, workspaceDir?: string): Promise<string> {
    const agentLogger = logger.child({
      taskId: this.taskId,
      component: 'ClaudeCodeAgent.sendMessage',
    })

    // Extract message from content
    const message = this.extractMessage(request)

    try {
      const sdkOptions = await this.buildSDKOptions(request, workspaceDir || process.cwd())
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
        agentLogger.debug({ sdkMessage }, 'Received SDK message')

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
      agentLogger.error({ err: error as Error }, 'Error in sendMessage')
      throw error
    }
  }
}
