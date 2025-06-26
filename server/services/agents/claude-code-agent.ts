import { query } from '@anthropic-ai/claude-code'
import type { ClaudeCodeOptions } from '@shared/types/claude.js'
import { type Event, EventType, type ResultEvent } from '@shared/types/event.js'
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
      const sdkOptions = this.buildSDKOptions(runOptions)

      // Use the SDK query function
      for await (const sdkMessage of query({
        prompt: message,
        abortController: this.abortController,
        options: sdkOptions,
      })) {
        if (this.stopRequested) {
          // TODO: Handle abort properly
          break
        }

        if (!this.sessionId && sdkMessage.session_id) {
          this.sessionId = sdkMessage.session_id
        }

        if (sdkMessage.type === 'result') {
          yield this.createEvent(sdkMessage, EventType.RESULT, sdkMessage.is_error) as ResultEvent
        } else {
          yield this.createEvent(sdkMessage, EventType.MESSAGE)
        }
      }
    } catch (error) {
      throw error
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

  getAgentType(): string {
    return 'claude-code'
  }

  private buildSDKOptions(runOptions: AgentRunOptions): ClaudeCodeOptions {
    const options = this.options as ClaudeCodeOptions
    const sdkOptions: ClaudeCodeOptions = {
      allowedTools: [
        'Agent',
        'Edit',
        'Grep',
        'MultiEdit',
        'NotebookRead',
        'TodoRead',
        'WebFetch',
        'Write',
        'WebSearch',
        'TodoWrite',
        'Read',
        'NotebookEdit',
        'LS',
        'Glob',
        'Bash',
      ],
      permissionMode: 'acceptEdits',
      ...options,
      cwd: runOptions.workspaceDir || options.cwd || process.cwd(),
    }

    if (runOptions.resume) {
      sdkOptions.resume = runOptions.resume
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
