import path from 'node:path'
import { query } from '@anthropic-ai/claude-agent-sdk'
import type {
  AgentEvent,
  BranchNameAndSummary,
  ClaudeCodeOptions,
  ClaudeCodeSDKMessage,
  Event,
  McpServerConfig,
} from '@moatless/types'

import { getGitHubService } from '../../services/github.service.js'
import { getSettingsService } from '../../services/settings.service.js'
import { calculateContextTokens } from '../../utils/context-tokens.js'
import { getDataDir } from '../../utils/data-dir.js'
import { parseBranchNameAndSummary } from '../../utils/json-parser.js'
import { logger } from '../../utils/logger.js'
import { createBranchNamePrompt } from '../../utils/prompts.js'
import { type AgentRunOptions, BaseAgent } from '../base-agent.js'

export class ClaudeCodeAgent extends BaseAgent {
  private abortController: AbortController | undefined
  private extractedPlan: string | undefined

  constructor(taskId: string, options: ClaudeCodeOptions = {}) {
    super(taskId, options)
  }

  async *run(message: string, runOptions: AgentRunOptions = {}): AsyncGenerator<Event> {
    this.stopRequested = false
    this.abortController = new AbortController()
    this.iterationNumber = runOptions.iterationNumber

    const agentLogger = this.createLogger(runOptions)
    this.logRunStart(agentLogger, runOptions)

    try {
      const sdkOptions = await this.buildSDKOptions(runOptions)
      this.logSDKOptions(agentLogger, sdkOptions, runOptions)

      let eventCount = 0
      let hasStartedProcessing = false
      let hasEmittedStarted = false

      agentLogger.debug('Initiating Claude Code SDK query', {
        promptLength: message.length,
        taskId: this.taskId,
      })

      for await (const sdkMessage of query({
        prompt: message,
        options: { ...sdkOptions, abortController: this.abortController },
      })) {
        eventCount++
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

      agentLogger.error('Unexpected Claude Code process exit', error, {
        stopRequested: this.stopRequested,
        sessionId: this.sessionId,
        exitCode,
      })

      yield this.createEventWithContextTokens(
        { type: 'agent', subType: 'error', error: error.message, exitCode },
        'agent',
        true,
      )

      throw error
    }

    agentLogger.error('Error in SDK query loop', error as Error, {
      stopRequested: this.stopRequested,
      sessionId: this.sessionId,
    })

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

  private async buildSDKOptions(runOptions: AgentRunOptions): Promise<ClaudeCodeOptions> {
    const options = this.options as ClaudeCodeOptions
    const githubService = getGitHubService()
    const settingsService = getSettingsService(githubService)
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

    // Get upload directory to add to additional directories
    const dataDir = getDataDir()
    const uploadDir = process.env['UPLOAD_DIR'] || path.join(dataDir, 'uploads')

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
      )
    }

    const sdkOptions: ClaudeCodeOptions = {
      // allowedTools,
      permissionMode,
      ...options,
      cwd: runOptions.workspaceDir || options.cwd || process.cwd(),
      additionalDirectories: [uploadDir],
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

      agentLogger.info('Added user-configured MCP servers', {
        userServerCount: Object.keys(mcpSettings.mcpServers).length,
        userServerNames: Object.keys(mcpSettings.mcpServers),
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
