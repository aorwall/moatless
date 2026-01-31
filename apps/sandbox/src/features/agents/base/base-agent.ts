import type { AgentRunRequest, FileAttachment } from '@moatless/sandbox-schemas'
import type { Event } from '@moatless/types'
import type { SandboxTool } from '../../tools/types'

export interface AgentOptions {
  model?: string
  apiKey?: string
  maxTokens?: number
  temperature?: number
  timeout?: number
  sandboxTools?: SandboxTool[]
  [key: string]: unknown
}

// Re-export types from shared package
export type { AgentRunRequest, FileAttachment }

/**
 * Callback for receiving agent events during execution.
 * This is NOT part of AgentRunRequest since callbacks cannot be serialized.
 */
export type AgentEventCallback = (event: Event) => void | Promise<void>

export abstract class BaseAgent {
  protected stopRequested = false
  protected sessionId: string | undefined
  protected onEvent: AgentEventCallback | undefined

  protected constructor(
    protected taskId: string,
    protected options: AgentOptions = {},
  ) {}

  /**
   * Run the agent with the given request.
   *
   * @param request - Agent run request containing content and options
   * @param workspaceDir - Workspace directory path (derived locally by sandbox, not in serializable request)
   * @param onEvent - Optional callback for receiving agent events
   */
  abstract run(
    request: AgentRunRequest,
    workspaceDir: string,
    onEvent?: AgentEventCallback,
  ): Promise<void>

  abstract stop(): Promise<void>
  abstract getAgentType(): string

  protected emitEvent(event: Event): void {
    this.onEvent?.(event)
  }

  protected delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
