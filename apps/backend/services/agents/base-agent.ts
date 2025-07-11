import type { ClaudeCodeSDKMessage } from '@moatless/shared/types/claude'
import type { Event, EventType } from '@moatless/shared/types/event'

export interface AgentOptions {
  model?: string
  apiKey?: string
  maxTokens?: number
  temperature?: number
  timeout?: number
  [key: string]: unknown
}

export interface AgentRunOptions {
  workspaceDir?: string
  environmentVariables?: Record<string, string>
  timeout?: number
  onProgress?: (event: Event) => void
  iterationNumber?: number
  resume?: string
}

export abstract class BaseAgent {
  protected stopRequested = false
  protected iterationNumber?: number
  protected sessionId?: string

  constructor(
    protected taskId: string,
    protected options: AgentOptions = {},
  ) {}

  abstract run(message: string, options?: AgentRunOptions): AsyncGenerator<Event>
  abstract stop(): Promise<void>
  abstract getAgentType(): string

  getSessionId(): string | undefined {
    return this.sessionId
  }

  protected createEvent(
    payload: ClaudeCodeSDKMessage,
    type: EventType,
    isError: boolean = false,
  ): Event {
    // Calculate contextTokens from usage data if available
    let contextTokens: number | undefined

    if (payload.type === 'assistant' && payload.message && 'usage' in payload.message) {
      const usage = payload.message.usage as Record<string, unknown> | null
      if (usage) {
        const inputTokens = Number(usage.input_tokens || 0)
        const cacheReadTokens = Number(usage.cache_read_input_tokens || 0)
        const cacheCreationTokens = Number(usage.cache_creation_input_tokens || 0)
        contextTokens = inputTokens + cacheReadTokens + cacheCreationTokens
      }
    }

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

  protected async delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
