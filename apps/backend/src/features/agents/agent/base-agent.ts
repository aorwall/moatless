import type {
  AgentRunRequest as BaseAgentRunRequest,
  FileAttachment,
} from '@moatless/sandbox-schemas'
import type { Event } from '@moatless/types'

export interface AgentOptions {
  model?: string
  apiKey?: string
  maxTokens?: number
  temperature?: number
  timeout?: number
  [key: string]: unknown
}

// Re-export FileAttachment from shared package
export type { FileAttachment }

/**
 * Unified request object for agent execution.
 * Extends the shared schema with the local-only `onEvent` callback
 * that cannot be serialized for HTTP transport.
 */
export interface AgentRunRequest extends BaseAgentRunRequest {
  onEvent?: (event: Event) => void | Promise<void>
}

/**
 * @deprecated Use AgentRunRequest instead
 */
export type AgentRunOptions = AgentRunRequest

export abstract class BaseAgent {
  protected stopRequested = false
  protected iterationNumber: number | undefined
  protected sessionId: string | undefined
  protected onEvent: ((event: Event) => void | Promise<void>) | undefined

  protected constructor(
    protected taskId: string,
    protected options: AgentOptions = {},
  ) {}

  /**
   * Run the agent with the given message or request.
   *
   * Supports two calling conventions:
   * 1. Unified request: run({ content: [...], ...options })
   * 2. Legacy: run(message, options) - message string with separate options
   *
   * Implementations should check if first argument is a string to determine the convention.
   *
   * @param messageOrRequest - Either a message string or AgentRunRequest object
   * @param options - Optional run options (only used when first arg is a string)
   */
  abstract run(messageOrRequest: string | AgentRunRequest, options?: AgentRunOptions): Promise<void>

  abstract stop(): Promise<void>
  abstract getAgentType(): string

  protected emitEvent(event: Event): void {
    this.onEvent?.(event)
  }

  protected delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
