import type { Event } from '@moatless/types'
import type { logger } from '../utils/logger.js'
import type { AgentOptions, AgentRunOptions } from './base-agent.js'
import { BaseAgent as BaseAgentClass } from './base-agent.js'

export interface ProxyAgentConfig {
  taskId: string
  agentType: string
  workerPort: number
  sessionId?: string
  options?: AgentOptions
  logger: ReturnType<typeof logger.child>
}

export class ProxyAgent extends BaseAgentClass {
  private remoteSessionId?: string
  private readonly agentType: string
  private readonly workerPort: number
  private readonly logger: ReturnType<typeof logger.child>

  constructor(config: ProxyAgentConfig) {
    super(config.taskId, config.options)
    this.agentType = config.agentType
    this.workerPort = config.workerPort
    this.remoteSessionId = config.sessionId
    this.logger = config.logger
  }

  async *run(message: string, runOptions?: AgentRunOptions): AsyncGenerator<Event> {
    const workerUrl = `http://localhost:${this.workerPort}/execute-agent`

    try {
      // Make HTTP request to worker to execute agent

      this.logger.info('Executing agent via proxy', { workerUrl })

      const response = await fetch(workerUrl, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Accept: 'text/event-stream',
        },
        body: JSON.stringify({
          agentType: this.agentType,
          taskId: this.taskId,
          message,
          model: runOptions?.model,
          runOptions: {
            ...runOptions,
            workspaceDir: '/repo',
          },
        }),
      })
      this.logger.info('Agent execution response', { response })
      if (!response.ok) {
        const error = await response.text()
        throw new Error(`Failed to execute agent in worker: ${error}`)
      }

      // Parse SSE stream from worker
      const reader = response.body?.getReader()
      const decoder = new TextDecoder()

      if (!reader) {
        throw new Error('No response body')
      }

      let buffer = ''

      while (true) {
        const { done, value } = await reader.read()
        if (done) break

        buffer += decoder.decode(value, { stream: true })
        const lines = buffer.split('\n')
        buffer = lines.pop() || ''

        for (const line of lines) {
          if (line.startsWith('data: ')) {
            const data = line.slice(6)
            if (data === '[DONE]') {
              return
            }
            try {
              const parsedData = JSON.parse(data)

              // Check if this is a session event (special case not in EventType enum)
              if (parsedData.type === 'session' && parsedData.sessionId) {
                this.remoteSessionId = parsedData.sessionId
              }

              // Yield the event regardless of type
              const event = parsedData as Event

              yield event
            } catch (error) {
              this.logger.error('Failed to parse event from worker', error as Error, { data })
            }
          }
        }
      }
    } catch (error) {
      this.logger.error('Error executing agent via proxy', error as Error, { workerUrl })
      throw error
    }
  }

  async stop(): Promise<void> {
    try {
      const stopUrl = `http://localhost:${this.workerPort}/stop-agent`
      const response = await fetch(stopUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ taskId: this.taskId }),
      })

      if (!response.ok) {
        const error = await response.text()
        throw new Error(`Failed to stop agent in worker: ${error}`)
      }
    } catch (error) {
      this.logger.error('Error stopping remote agent', error as Error)
      throw error
    }
  }

  getAgentType(): string {
    return this.agentType
  }

  getSessionId(): string | undefined {
    return this.remoteSessionId
  }

  async getBranchNameAndSummary(message: string, options?: AgentRunOptions): Promise<null> {
    try {
      const branchUrl = `http://localhost:${this.workerPort}/agent-branch-summary`
      const response = await fetch(branchUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ taskId: this.taskId, message, options }),
      })

      if (!response.ok) {
        const error = await response.text()
        throw new Error(`Failed to get branch summary from agent: ${error}`)
      }

      const result = (await response.json()) as { success: boolean; result: null }
      return result.result
    } catch (error) {
      this.logger.error('Error getting branch summary from remote agent', error as Error)
      throw error
    }
  }
}
