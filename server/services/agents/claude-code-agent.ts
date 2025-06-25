import { query, type SDKMessage } from '@anthropic-ai/claude-code'
import type { ClaudeCodeOptions } from '@shared/types/claude.js'
import type { Event } from '@shared/types/event.js'
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

    try {
      yield this.createEvent('start', {
        message,
        agentType: 'claude-code',
        options: this.options,
      })

      const sdkOptions = this.buildSDKOptions(runOptions)
      const messages: SDKMessage[] = []
      let lastEvent: Event | null = null
      let hasError = false

      // Use the SDK query function
      for await (const sdkMessage of query({
        prompt: message,
        abortController: this.abortController,
        options: sdkOptions,
      })) {
        if (this.stopRequested) {
          yield this.createEvent('stop', { reason: 'user_requested' })
          break
        }

        // Check if this is the first system message with init subtype
        if (
          sdkMessage.type === 'system' &&
          sdkMessage.subtype === 'init' &&
          sdkMessage.session_id
        ) {
          this.sessionId = sdkMessage.session_id
        }

        messages.push(sdkMessage)

        // Detect if this is a tool call or tool result message
        let subType: string | undefined
        if (sdkMessage.type === 'assistant' && sdkMessage.message?.content) {
          const content = sdkMessage.message.content
          if (Array.isArray(content) && content.some((item) => item.type === 'tool_use')) {
            subType = 'tool_call'
          }
        } else if (sdkMessage.type === 'user' && sdkMessage.message?.content) {
          const content = sdkMessage.message.content
          if (Array.isArray(content) && content.some((item) => item.type === 'tool_result')) {
            subType = 'tool_result'
          }
        }

        const event = this.createEvent('message', sdkMessage, false, subType)

        if (event.isError) {
          hasError = true
        }

        lastEvent = event
        yield event
      }

      // Generate completion event
      if (!hasError && !this.stopRequested) {
        yield this.createEvent('completion', {
          successful: true,
          messageCount: messages.length,
          lastEvent: lastEvent?.payload,
        })
      }
    } catch (error) {
      yield this.createEvent(
        'error',
        {
          error: error instanceof Error ? error.message : String(error),
          stack: error instanceof Error ? error.stack : undefined,
        },
        true,
      )
    } finally {
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

  private buildSDKOptions(runOptions: AgentRunOptions): ClaudeCodeOptions {
    const options = this.options as ClaudeCodeOptions
    const sdkOptions: ClaudeCodeOptions = {
      // Copy all SDK options directly from our options
      ...options,
      // Override cwd with workspace directory if provided
      cwd: runOptions.workspaceDir || options.cwd || process.cwd(),
      // Don't pass abortController through options - it's handled separately
      abortController: undefined,
    }

    return sdkOptions
  }

  private async cleanup(): Promise<void> {
    if (this.abortController && !this.abortController.signal.aborted) {
      this.abortController.abort()
    }
    this.abortController = undefined
  }
}
