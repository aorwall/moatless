import type { BranchNameAndSummary, Event, MessageContext } from '@moatless/types'

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
  model?: string
  agentMode?: 'edit' | 'plan'
  mcpServerUrl?: string
  hasPreviewServer?: boolean
  appendSystemPrompt?: string
  // User message metadata for message persistence
  fileIds?: string[]
  contexts?: MessageContext[]
}

export abstract class BaseAgent {
  protected stopRequested = false
  protected iterationNumber: number | undefined
  protected sessionId: string | undefined

  protected constructor(
    protected taskId: string,
    protected options: AgentOptions = {},
  ) {}

  abstract run(message: string, options?: AgentRunOptions): AsyncGenerator<Event>
  abstract stop(): Promise<void>
  abstract getAgentType(): string
  abstract getBranchNameAndSummary(
    message: string,
    options?: AgentRunOptions,
  ): Promise<BranchNameAndSummary | null>

  getSessionId(): string | undefined {
    return this.sessionId
  }

  protected delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
