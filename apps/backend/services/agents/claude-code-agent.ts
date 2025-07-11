import { query } from '@anthropic-ai/claude-code'
import type { ClaudeCodeOptions, ClaudeCodeSDKMessage } from '@moatless/shared/types/claude'
import { type Event, EventType, type ResultEvent } from '@moatless/shared/types/event'
import { calculateContextTokens } from '../../utils/context-tokens.js'
import { logger } from '../../utils/logger.js'
import { type AgentRunOptions, BaseAgent } from './base-agent.js'

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

    try {
      const sdkOptions = this.buildSDKOptions(runOptions)

      let eventCount = 0
      let hasStartedProcessing = false

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

        if (this.stopRequested) {
          agentLogger.info('Stop requested, breaking from SDK query loop', { eventCount })
          break
        }

        if (!this.sessionId && sdkMessage.session_id) {
          this.sessionId = sdkMessage.session_id
          agentLogger.debug('Session ID captured from SDK', { sessionId: this.sessionId })
        }

        // Mark that we've started processing beyond init
        if (sdkMessage.type !== 'system' || hasStartedProcessing) {
          hasStartedProcessing = true
        }

        if (sdkMessage.type === 'result') {
          const isError = 'is_error' in sdkMessage ? sdkMessage.is_error : false
          yield this.createEventWithContextTokens(
            sdkMessage,
            EventType.RESULT,
            isError,
          ) as ResultEvent
        } else {
          yield this.createEventWithContextTokens(sdkMessage, EventType.MESSAGE)
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
        const exitCode = exitCodeMatch ? parseInt(exitCodeMatch[1]) : null

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

        // Only throw if it's truly unexpected
        throw error
      } else {
        agentLogger.error('Error in SDK query loop', error as Error, {
          stopRequested: this.stopRequested,
          sessionId: this.sessionId,
        })
        throw error
      }
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
    payload: ClaudeCodeSDKMessage,
    type: EventType,
    isError: boolean = false,
  ): Event {
    const contextTokens = calculateContextTokens(payload)

    return {
      eventId: crypto.randomUUID(),
      agentType: this.getAgentType(),
      type,
      taskId: this.taskId,
      iterationId: this.iterationNumber,
      timestamp: new Date(),
      isError,
      contextTokens,
      payload,
    }
  }

  private buildSDKOptions(runOptions: AgentRunOptions): ClaudeCodeOptions {
    const options = this.options as ClaudeCodeOptions
    const sdkOptions: ClaudeCodeOptions = {
      allowedTools: ['WebFetch', 'WebSearch', 'Bash'],
      permissionMode: 'acceptEdits',
      ...options,
      cwd: runOptions.workspaceDir || options.cwd || process.cwd(),
    }

    // Add model if provided
    if (options.model) {
      sdkOptions.model = options.model
    }

    if (runOptions.resume) {
      sdkOptions.resume = runOptions.resume
    }

    const agentLogger = logger.child({
      taskId: this.taskId,
      component: 'ClaudeCodeAgent',
    })

    agentLogger.debug('Built SDK options', {
      sdkOptions,
    })

    return sdkOptions
  }

  private async cleanup(): Promise<void> {
    if (this.abortController && !this.abortController.signal.aborted) {
      this.abortController.abort()
    }
    this.abortController = undefined
  }
}
