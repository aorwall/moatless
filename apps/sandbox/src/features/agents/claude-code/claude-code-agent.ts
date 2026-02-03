import { randomUUID } from 'node:crypto'
import { query } from '@anthropic-ai/claude-agent-sdk'
import type { AgentEvent, ClaudeCodeOptions, ClaudeCodeSDKMessage } from '@moatless/types'
import type { Logger } from 'pino'
import { getAgentConfig } from '../../../config/environment'
import { logger } from '../../../shared/logger'
import {
  type AgentEventCallback,
  type AgentOptions,
  type AgentRunRequest,
  BaseAgent,
} from '../base/base-agent'
import { createAgentEvent, createResultEvent } from './event-helpers'
import {
  createSDKMessageStream,
  extractMessage,
  getSDKMessageLogInfo,
  logErrorContentItems,
  withFirstMessageTimeout,
} from './sdk-message-helpers'
import { buildSDKOptions, type SDKBuildContext } from './sdk-options-builder'

export class ClaudeCodeAgent extends BaseAgent {
  private abortController: AbortController | undefined
  private extractedPlan: string | undefined
  private lastSdkMessage: ClaudeCodeSDKMessage | undefined
  private stderrOutput: string[] = []
  private lastProcessExitInfo: { code: number | null; signal: string | null } | null = null

  constructor(taskId: string, options: AgentOptions = {}) {
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
    this.lastProcessExitInfo = null

    const agentLogger = this.createLogger(request)

    // Validate resume session ID consistency
    if (typeof request.resume === 'string' && this.sessionId && request.resume !== this.sessionId) {
      agentLogger.warn(
        {
          requestResumeSessionId: request.resume,
          currentSessionId: this.sessionId,
        },
        'Resume session ID differs from current agent session ID',
      )
    }
    if (request.resume === true && !this.sessionId) {
      agentLogger.warn(
        'resume=true but no session ID set on agent, will rely on SDK to handle resume',
      )
    }

    agentLogger.info(
      {
        taskId: this.taskId,
        hasResumeSession: Boolean(request.resume),
        workspaceDir,
      },
      'Starting Claude Code agent execution',
    )

    let hasEmittedStarted = false

    try {
      const sdkOptions = await this.buildOptions(request, workspaceDir)
      this.logSDKOptions(agentLogger, sdkOptions, request)

      let eventCount = 0
      let hasStartedProcessing = false
      let hasEmittedCompletion = false

      const sessionId = this.sessionId || randomUUID()

      agentLogger.debug(
        {
          sessionId,
          taskId: this.taskId,
        },
        'Initiating Claude Code SDK query',
      )

      const prompt = createSDKMessageStream([request], sessionId)

      const queryIterable = withFirstMessageTimeout(
        query({
          prompt,
          options: { ...sdkOptions, abortController: this.abortController },
        }),
        this.abortController,
        agentLogger,
        this.stderrOutput,
        this.lastProcessExitInfo,
      )

      for await (const sdkMessage of queryIterable) {
        eventCount++
        this.lastSdkMessage = sdkMessage
        this.logSDKMessage(agentLogger, sdkMessage, eventCount)

        if (!hasEmittedStarted) {
          hasEmittedStarted = true
          this.emitEvent(this.createEvent({ type: 'agent', subType: 'started' }, false))
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
          this.emitEvent(this.createEvent(sdkMessage, false, 'message'))
        }
      }

      if (this.stopRequested && !hasEmittedCompletion && hasEmittedStarted) {
        agentLogger.info('Emitting stopped event for user-initiated stop')
        this.emitEvent(this.createEvent({ type: 'agent', subType: 'stopped' }, false))
      }

      this.logQueryCompletion(agentLogger, eventCount, hasStartedProcessing)
    } catch (error) {
      this.handleError(agentLogger, error, hasEmittedStarted)
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

  async sendMessage(request: AgentRunRequest, workspaceDir?: string): Promise<string> {
    const agentLogger = logger.child({
      taskId: this.taskId,
      component: 'ClaudeCodeAgent.sendMessage',
    })

    const message = extractMessage(request)

    try {
      const sdkOptions = await this.buildOptions(request, workspaceDir || process.cwd())
      const abortController = new AbortController()

      let responseText = ''

      for await (const sdkMessage of query({
        prompt: message,
        options: {
          ...sdkOptions,
          abortController,
        },
      })) {
        agentLogger.debug({ sdkMessage }, 'Received SDK message')

        if (sdkMessage.type === 'result' && 'result' in sdkMessage && sdkMessage.result) {
          responseText += sdkMessage.result
        } else if (
          (sdkMessage.type === 'assistant' ||
            sdkMessage.type === 'user' ||
            sdkMessage.type === 'system') &&
          'content' in sdkMessage &&
          sdkMessage.content
        ) {
          responseText += sdkMessage.content
        }
      }

      return responseText
    } catch (error) {
      agentLogger.error({ err: error as Error }, 'Error in sendMessage')
      throw error
    }
  }

  // ===== Private helpers =====

  private createLogger(_request: AgentRunRequest) {
    return logger.child({
      taskId: this.taskId,
      component: 'ClaudeCodeAgent',
    })
  }

  private get eventOpts() {
    return {
      agentType: this.getAgentType(),
      taskId: this.taskId,
      sessionId: this.sessionId,
    }
  }

  private createEvent(
    payload: ClaudeCodeSDKMessage | Record<string, unknown>,
    isError: boolean,
    subType?: string,
  ): AgentEvent {
    return createAgentEvent(payload, { ...this.eventOpts, isError, subType })
  }

  private async buildOptions(
    request: AgentRunRequest,
    workspaceDir: string,
  ): Promise<ClaudeCodeOptions> {
    const ctx: SDKBuildContext = {
      request,
      workspaceDir,
      options: this.options,
      taskId: this.taskId,
      sessionId: this.sessionId,
      agentType: this.getAgentType(),
      stderrCollector: (data: string) => {
        this.stderrOutput.push(data)
      },
      processExitCollector: (info) => {
        this.lastProcessExitInfo = info
      },
      emitEvent: (event: AgentEvent) => this.emitEvent(event),
    }
    return await buildSDKOptions(ctx)
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
    const logInfo = getSDKMessageLogInfo(sdkMessage)
    agentLogger.info({ eventCount }, `SDK Event: type=${sdkMessage.type}${logInfo}`)
    logErrorContentItems(agentLogger, sdkMessage, eventCount)
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

    const { event, planConsumed } = createResultEvent(
      agentLogger,
      sdkMessage,
      isError as boolean,
      this.extractedPlan,
      this.eventOpts,
    )
    if (planConsumed) {
      this.extractedPlan = undefined
    }
    this.emitEvent(event)
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
      throw new Error(errorMessage)
    }
  }

  private handleError(agentLogger: Logger, error: unknown, hasEmittedStarted = false): void {
    const errorMessage = error instanceof Error ? error.message : String(error)
    const errorName = error instanceof Error ? error.name : 'Unknown'

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
        agentLogger.warn(
          {
            sessionId: this.sessionId,
            errorMessage,
            errorName,
          },
          'Agent stopped with unexpected error type (stop was requested)',
        )
      }

      if (hasEmittedStarted) {
        agentLogger.info('Emitting stopped event from error handler')
        this.emitEvent(this.createEvent({ type: 'agent', subType: 'stopped' }, false))
      }

      return
    }

    if (error instanceof Error && error.message.includes('Claude Code process exited')) {
      const exitCodeMatch = error.message.match(/code (\d+)/)
      const exitCode = exitCodeMatch?.[1] ? Number.parseInt(exitCodeMatch[1], 10) : null
      const isExpectedExitCode = exitCode === 143 || exitCode === 130

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

      this.logDetailedErrorInfo(agentLogger, error, exitCode)
      this.emitStderrMessageEvent(agentLogger)

      this.emitEvent(
        this.createEvent(
          {
            type: 'agent',
            subType: 'error',
            error: error.message,
            exitCode,
          },
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

    this.emitStderrMessageEvent(agentLogger)

    if (errorMessage.includes('No SDK messages received')) {
      this.emitEvent(
        this.createEvent(
          {
            type: 'agent',
            subType: 'message',
            message: {
              type: 'message',
              role: 'assistant',
              content: [
                {
                  type: 'text',
                  text: `❌ **Agent Error**\n\nThe Claude Code CLI started but exited immediately without responding.\n\n**Exit info:** ${JSON.stringify(this.lastProcessExitInfo)}\n**stderr:** ${this.stderrOutput.join('\n') || 'none'}\n\nThis usually indicates an authentication or network issue.`,
                },
              ],
            },
          },
          false,
          'message',
        ),
      )
    }

    this.emitEvent(
      this.createEvent(
        {
          type: 'agent',
          subType: 'error',
          error: error instanceof Error ? error.message : String(error),
          stackTrace: error instanceof Error ? error.stack : undefined,
        },
        true,
      ),
    )

    throw error
  }

  /**
   * Parse stderr output to extract meaningful error messages
   */
  private parseStderrError(): string | null {
    if (this.stderrOutput.length === 0) {
      return null
    }

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
      this.createEvent(
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

  // biome-ignore lint/suspicious/useAwait: cleanup is awaited elsewhere for consistency
  private async cleanup(): Promise<void> {
    if (this.abortController && !this.abortController.signal.aborted) {
      this.abortController.abort()
    }
    this.abortController = undefined
  }
}
