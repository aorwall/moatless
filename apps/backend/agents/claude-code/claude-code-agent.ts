import { query } from '@anthropic-ai/claude-code'
import type {
  BranchNameAndSummary,
  ClaudeCodeOptions,
  ClaudeCodeSDKMessage,
  Event,
  EventTypeSchema,
  ResultEvent,
} from '@moatless/types'
import type { z } from 'zod'

import { getGitHubService } from '../../services/github.service.js'
import { getSettingsService } from '../../services/settings.service.js'
import { calculateContextTokens } from '../../utils/context-tokens.js'
import { parseBranchNameAndSummary } from '../../utils/json-parser.js'
import { logger } from '../../utils/logger.js'
import { createBranchNamePrompt } from '../../utils/prompts.js'
import { type AgentRunOptions, BaseAgent } from '../base-agent.js'

export class ClaudeCodeAgent extends BaseAgent {
  private abortController?: AbortController

  constructor(taskId: string, options: ClaudeCodeOptions = {}) {
    super(taskId, options)
  }

  async *run(message: string, runOptions: AgentRunOptions = {}): AsyncGenerator<Event> {
    this.stopRequested = false
    this.abortController = new AbortController()
    this.iterationNumber = runOptions.iterationNumber

    const agentLogger = logger.child({
      taskId: this.taskId,
      iterationNumber: runOptions.iterationNumber,
      component: 'ClaudeCodeAgent',
    })

    agentLogger.info('Starting Claude Code agent execution', {
      taskId: this.taskId,
      iterationNumber: runOptions.iterationNumber,
      hasResumeSession: !!runOptions.resume,
      workspaceDir: runOptions.workspaceDir,
    })

    try {
      const sdkOptions = this.buildSDKOptions(runOptions)

      agentLogger.debug('SDK options built', {
        model: sdkOptions.model,
        allowedTools: sdkOptions.allowedTools,
        permissionMode: sdkOptions.permissionMode,
        cwd: sdkOptions.cwd,
        resume: sdkOptions.resume,
      })

      let eventCount = 0
      let hasStartedProcessing = false
      let hasEmittedStarted = false

      agentLogger.info('Initiating Claude Code SDK query', {
        promptLength: message.length,
        taskId: this.taskId,
      })

      // Use the SDK query function
      for await (const sdkMessage of query({
        prompt: message,
        abortController: this.abortController,
        options: sdkOptions,
      })) {
        eventCount++

        agentLogger.debug('Received SDK message', {
          eventCount,
          sdkMessage,
        })

        // Emit 'agent started' event on first real event (not system/init)
        if (!hasEmittedStarted && sdkMessage.type !== 'system') {
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

        if (!this.sessionId && sdkMessage.session_id) {
          this.sessionId = sdkMessage.session_id
          agentLogger.info('Session ID captured from SDK', { sessionId: this.sessionId })
        }

        // Mark that we've started processing beyond init
        if (sdkMessage.type !== 'system' || hasStartedProcessing) {
          hasStartedProcessing = true
        }

        if (sdkMessage.type === 'result') {
          const isError = 'is_error' in sdkMessage ? sdkMessage.is_error : false
          agentLogger.info('Received result message from SDK', {
            isError,
            subtype: 'subtype' in sdkMessage ? sdkMessage.subtype : undefined,
          })
          yield this.createEventWithContextTokens(sdkMessage, 'result', isError) as ResultEvent
        } else {
          yield this.createEventWithContextTokens(sdkMessage, 'message')
        }
      }

      agentLogger.info('SDK query loop completed', {
        totalEvents: eventCount,
        hasStartedProcessing,
        stopRequested: this.stopRequested,
        sessionId: this.sessionId,
      })

      // Log warning if we only got init message
      if (eventCount === 1 && !hasStartedProcessing) {
        agentLogger.warn(
          'SDK query loop exited after only init message - this may indicate an authentication or configuration issue',
        )
      }
    } catch (error) {
      // Check if it's a process exit error during normal operation
      if (error instanceof Error && error.message.includes('Claude Code process exited')) {
        // Parse exit code from error message if available
        const exitCodeMatch = error.message.match(/code (\d+)/)
        const exitCode = exitCodeMatch ? Number.parseInt(exitCodeMatch[1]) : null

        // Exit codes 143 (SIGTERM) and 130 (SIGINT) are expected when stopping
        const isExpectedExitCode = exitCode === 143 || exitCode === 130

        // If we've requested stop or it's an expected exit code, this is normal behavior
        if (this.stopRequested || isExpectedExitCode) {
          agentLogger.info('Claude Code process exited after stop request', {
            exitCode,
            stopRequested: this.stopRequested,
            sessionId: this.sessionId,
          })
          return
        }

        agentLogger.error('Unexpected Claude Code process exit', error as Error, {
          stopRequested: this.stopRequested,
          sessionId: this.sessionId,
          exitCode,
        })

        // Emit agent error event
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

        // Only throw if it's truly unexpected
        throw error
      }
      agentLogger.error('Error in SDK query loop', error as Error, {
        stopRequested: this.stopRequested,
        sessionId: this.sessionId,
      })

      // Emit agent error event
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

  private createEventWithContextTokens(
    payload: ClaudeCodeSDKMessage | Record<string, unknown>,
    type: z.infer<typeof EventTypeSchema>,
    isError = false,
  ): Event {
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
      subType: 'subType' in payload ? (payload.subType as string) : undefined,
      taskId: this.taskId,
      iterationId: this.iterationNumber,
      timestamp: new Date(),
      agentSessionId: sessionId,
      isError,
      contextTokens,
      payload,
    }
  }

  private buildSDKOptions(runOptions: AgentRunOptions): ClaudeCodeOptions {
    const options = this.options as ClaudeCodeOptions
    const githubService = getGitHubService()
    const settingsService = getSettingsService(githubService)
    const claudeSettings = settingsService.getClaudeSettings()

    const agentLogger = logger.child({
      taskId: this.taskId,
      component: 'ClaudeCodeAgent',
    })

    agentLogger.debug('Building SDK options', {
      hasClaudeSettings: !!claudeSettings,
      hasBinDir: !!claudeSettings.binDir,
      hasConfigDir: !!claudeSettings.configDir,
    })

    // Set environment variables for Claude Code if provided by settings
    if (claudeSettings.binDir && claudeSettings.binDir.trim() !== '') {
      process.env.CLAUDE_BIN_DIR = claudeSettings.binDir
      agentLogger.debug('Set CLAUDE_BIN_DIR environment variable', {
        binDir: claudeSettings.binDir,
      })
    }
    if (claudeSettings.configDir && claudeSettings.configDir.trim() !== '') {
      process.env.CLAUDE_CONFIG_DIR = claudeSettings.configDir
      agentLogger.debug('Set CLAUDE_CONFIG_DIR environment variable', {
        configDir: claudeSettings.configDir,
      })
    }

    const sdkOptions: ClaudeCodeOptions = {
      allowedTools: ['WebFetch', 'WebSearch', 'Bash'],
      permissionMode: 'acceptEdits',
      ...options,
      cwd: runOptions.workspaceDir || options.cwd || process.cwd(),
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

    agentLogger.info('Built SDK options successfully', {
      model: sdkOptions.model,
      cwd: sdkOptions.cwd,
      hasResumeSession: !!sdkOptions.resume,
      allowedToolsCount: sdkOptions.allowedTools?.length,
    })

    return sdkOptions
  }

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
      const sdkOptions = this.buildSDKOptions(runOptions)
      const abortController = new AbortController()

      let responseText = ''

      // Use the SDK query function
      for await (const sdkMessage of query({
        prompt: message,
        abortController,
        options: sdkOptions,
      })) {
        agentLogger.debug('Received SDK message', { sdkMessage })

        if (sdkMessage.type === 'result') {
          // For result messages, use the result field
          if ('result' in sdkMessage && sdkMessage.result) {
            responseText += sdkMessage.result
          }
        } else if (
          sdkMessage.type === 'assistant' ||
          sdkMessage.type === 'user' ||
          sdkMessage.type === 'system'
        ) {
          // For message types, use the content field
          if ('content' in sdkMessage && sdkMessage.content) {
            responseText += sdkMessage.content
          }
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
      const sdkOptions = this.buildSDKOptions(runOptions)

      // Override options for this specific task
      const branchNameOptions: ClaudeCodeOptions = {
        ...sdkOptions,
        maxTurns: 1,
        allowedTools: [],
        model: sdkOptions.model || 'claude-sonnet-4-20250514',
      }

      // Create the formatted prompt with the user's request
      const formattedPrompt = createBranchNamePrompt(message)

      const abortController = new AbortController()
      let responseText = ''

      // Use the SDK query function with branch name specific options
      for await (const sdkMessage of query({
        prompt: formattedPrompt,
        abortController,
        options: branchNameOptions,
      })) {
        agentLogger.debug('Received SDK message for branch name', { sdkMessage })

        if (sdkMessage.type === 'result') {
          // For result messages, use the result field
          if ('result' in sdkMessage && sdkMessage.result) {
            responseText += sdkMessage.result
          }
        } else if (
          sdkMessage.type === 'assistant' ||
          sdkMessage.type === 'user' ||
          sdkMessage.type === 'system'
        ) {
          // For message types, use the content field
          if ('content' in sdkMessage && sdkMessage.content) {
            responseText += sdkMessage.content
          }
        }
      }

      agentLogger.debug('Raw response for branch name', { response: responseText })

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
