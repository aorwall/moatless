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
}

export abstract class BaseAgent {
  protected stopRequested = false

  constructor(
    protected taskId: string,
    protected options: AgentOptions = {},
  ) {}

  abstract run(message: string, options?: AgentRunOptions): AsyncGenerator<Event>
  abstract stop(): Promise<void>

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
      timestamp: new Date(),
      isError,
      payload,
    }
  }

  protected async delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
