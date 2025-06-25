import type { Event } from '@shared/types/event.js'

export interface AgentOptions {
  model?: string
  apiKey?: string
  maxTokens?: number
  temperature?: number
  timeout?: number
  [key: string]: any
}

export interface AgentRunOptions {
  workspaceDir?: string
  environmentVariables?: Record<string, string>
  timeout?: number
  onProgress?: (event: Event) => void
  iterationNumber?: number
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

  getSessionId(): string | undefined {
    return this.sessionId
  }

  protected createEvent(
    type: string,
    payload: any,
    isError: boolean = false,
    subType?: string,
  ): Event {
    return {
      eventId: crypto.randomUUID(),
      type: type as any,
      subType,
      taskId: this.taskId,
      iterationId: this.iterationNumber,
      timestamp: new Date(),
      isError,
      payload,
    }
  }

  protected async delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
