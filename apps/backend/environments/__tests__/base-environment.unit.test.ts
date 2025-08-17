import type { Event, RepositoryConfig } from '@moatless/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { AgentOptions, BaseAgent } from '../../agents/base-agent.js'
import { BaseEnvironment } from '../base-environment.js'

// Mock external dependencies
vi.mock('../../utils/logger.js')

// Create a concrete implementation for testing
class TestEnvironment extends BaseEnvironment {
  private _isRunning = false
  public testStatus: 'idle' | 'running' = 'idle'
  private _status: 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning' = 'ready'
  private _error?: string
  private _createdAt = new Date()

  async createAgent(agentType: string, _options?: AgentOptions): Promise<BaseAgent> {
    const mockAgent: BaseAgent = {
      run: vi.fn().mockReturnValue(async function* (): AsyncGenerator<Event> {
        yield { type: 'message', agentType, content: 'test event' } as any
      }),
      stop: vi.fn().mockResolvedValue(undefined),
      getAgentType: () => agentType,
      getBranchNameAndSummary: vi.fn().mockResolvedValue(null),
      getSessionId: () => 'test-session-id',
    } as any

    this.currentAgent = mockAgent
    return mockAgent
  }

  async *runAgent(
    agentType: string,
    message: string,
    options?: { workspaceDir?: string; iterationNumber?: number; resume?: string; model?: string },
  ): AsyncGenerator<any> {
    const agent = await this.createAgent(agentType, options)
    this._isRunning = true
    this.testStatus = 'running'

    yield* agent.run(message, options)

    this._isRunning = false
    this.testStatus = 'idle'
  }

  async stopAgent(): Promise<boolean> {
    if (this.currentAgent) {
      await this.currentAgent.stop()
      this.currentAgent = undefined
      this._isRunning = false
      this.testStatus = 'idle'
      return true
    }
    return false
  }

  // Additional test methods
  isRunning(): boolean {
    return this._isRunning
  }

  // Implement all required abstract methods
  async startServer(): Promise<any> {
    return { status: 'running', port: 3000 }
  }

  async stopServer(): Promise<boolean> {
    return true
  }

  async getServerStatus(): Promise<any> {
    return { status: 'running', port: 3000 }
  }

  async getServerLogs(_lines?: number): Promise<string> {
    return 'Test logs'
  }

  getPreviewPort(): number | undefined {
    return 3000
  }

  async proxyPreviewRequest(_request: any): Promise<any> {
    return {
      status: 200,
      headers: new Headers(),
      body: 'Test response',
    }
  }

  async runCommand(_command: string, _timeout?: number): Promise<any> {
    return {
      success: true,
      output: 'Test output',
      error: '',
      exitCode: 0,
    }
  }

  async *runCommandWithStream(
    _command: string,
    _timeout?: number,
  ): AsyncGenerator<import('@moatless/types').CommandExecutionEvent> {
    yield {
      type: 'start',
      data: {
        timestamp: new Date().toISOString(),
        command: _command,
      },
    }
    yield {
      type: 'output',
      data: {
        timestamp: new Date().toISOString(),
        text: 'Test output',
        outputType: 'stdout',
      },
    }
    yield {
      type: 'complete',
      data: {
        timestamp: new Date().toISOString(),
        exitCode: 0,
        duration: 100,
      },
    }
  }

  async setup(_iterationNumber?: number): Promise<void> {
    this._status = 'ready'
  }

  async teardown(_force?: boolean): Promise<void> {
    this._status = 'cleaning'
  }

  getWorkspacePath(): string {
    return '/test/workspace'
  }

  getType(): 'local' | 'docker' {
    return 'local'
  }

  getStatus(): 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    return this._status
  }

  getError(): string | undefined {
    return this._error
  }

  getCreatedAt(): Date {
    return this._createdAt
  }
}

describe('BaseEnvironment - Unit Tests', () => {
  let testEnvironment: TestEnvironment
  let mockEventService: any

  const mockRepository: RepositoryConfig = {
    id: 'test-repo',
    name: 'Test Repository',
    path: '/test/path',
    codeExecution: { type: 'local' },
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  beforeEach(() => {
    vi.clearAllMocks()

    // Mock event service
    mockEventService = {
      emitEvent: vi.fn().mockResolvedValue(undefined),
    }

    testEnvironment = new TestEnvironment('test-task', mockRepository, mockEventService)
  })

  describe('Constructor and Base Properties', () => {
    it('should initialize with correct parameters', () => {
      // Can only test that construction doesn't throw
      expect(testEnvironment).toBeDefined()
      expect(testEnvironment).toBeInstanceOf(BaseEnvironment)
      expect(testEnvironment).toBeInstanceOf(TestEnvironment)
    })
  })

  describe('Agent Management', () => {
    it('should create and manage agents', async () => {
      const agent = await testEnvironment.createAgent('test-agent')

      expect(agent).toBeDefined()
      expect(agent.getAgentType()).toBe('test-agent')
      expect(agent.getSessionId()).toBe('test-session-id')
    })

    it('should get agent status when no agent exists', () => {
      const status = testEnvironment.getAgentStatus()

      expect(status).toEqual({
        state: 'none',
      })
    })

    it('should get agent status when agent exists', async () => {
      await testEnvironment.createAgent('test-agent')

      const status = testEnvironment.getAgentStatus()

      expect(status).toEqual({
        agentType: 'test-agent',
        state: 'running',
        sessionId: 'test-session-id',
      })
    })

    it('should run agent and yield events', async () => {
      const events = []

      for await (const event of testEnvironment.runAgent('test-agent', 'test message')) {
        events.push(event)
      }

      expect(events).toHaveLength(1)
      expect(events[0]).toEqual({
        type: 'message',
        agentType: 'test-agent',
        content: 'test event',
      })
    })

    it('should stop agent successfully', async () => {
      await testEnvironment.createAgent('test-agent')

      const stopped = await testEnvironment.stopAgent()

      expect(stopped).toBe(true)
      expect(testEnvironment.getAgentStatus().state).toBe('none')
    })

    it('should handle stop when no agent exists', async () => {
      const stopped = await testEnvironment.stopAgent()

      expect(stopped).toBe(false)
    })
  })

  describe('Event Service Integration', () => {
    it('should emit status change events when event service is provided', async () => {
      // Access protected method through type assertion for testing
      const env = testEnvironment as any

      await env.emitStatusChange('ready', 'initializing')

      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'test-task',
          type: 'environment',
          subType: 'environment:ready',
          payload: {
            status: 'ready',
            previousStatus: 'initializing',
            workspaceId: 'test-task',
          },
        }),
      )
    })

    it('should handle missing event service gracefully', async () => {
      const envWithoutEvents = new TestEnvironment('test-task', mockRepository)
      const env = envWithoutEvents as any

      // Should not throw
      await expect(env.emitStatusChange('ready', 'initializing')).resolves.not.toThrow()
    })
  })

  describe('Abstract Method Implementation', () => {
    it('should require implementation of abstract methods', () => {
      // Verify TestEnvironment implements all required methods
      expect(testEnvironment.createAgent).toBeDefined()
      expect(testEnvironment.runAgent).toBeDefined()
      expect(testEnvironment.stopAgent).toBeDefined()
      expect(testEnvironment.getAgentStatus).toBeDefined()
    })
  })

  describe('Multiple Environments', () => {
    it('should handle multiple independent instances', async () => {
      const env1 = new TestEnvironment('task-1', mockRepository, mockEventService)
      const env2 = new TestEnvironment('task-2', mockRepository, mockEventService)

      await env1.createAgent('agent-1')

      expect(env1.getAgentStatus().agentType).toBe('agent-1')
      expect(env2.getAgentStatus().state).toBe('none')
    })
  })
})
