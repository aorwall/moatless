import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createTaskLogger } from '../../../../utils/logger'
import { ProxyAgent } from '../proxy-agent'

// Mock external dependencies
vi.mock('../../utils/logger.js')

// Mock fetch globally
global.fetch = vi.fn()

const mockCreateTaskLogger = vi.mocked(createTaskLogger)

describe('ProxyAgent - Unit Tests', () => {
  let proxyAgent: ProxyAgent
  let mockLogger: any
  let mockFetch: any

  beforeEach(() => {
    vi.clearAllMocks()

    // Mock logger
    mockLogger = {
      info: vi.fn(),
      error: vi.fn(),
      debug: vi.fn(),
      warn: vi.fn(),
      child: vi.fn().mockReturnThis(),
    }
    mockCreateTaskLogger.mockReturnValue(mockLogger)

    // Store mocked fetch
    mockFetch = vi.mocked(global.fetch)

    // Create ProxyAgent instance with proper config
    proxyAgent = new ProxyAgent({
      taskId: 'test-task',
      agentType: 'test-agent',
      workerPort: 8001,
      logger: mockLogger,
    })
  })

  describe('Constructor', () => {
    it('should create instance successfully', () => {
      // Just verify the instance was created without errors
      expect(proxyAgent).toBeDefined()
      expect(proxyAgent).toBeInstanceOf(ProxyAgent)
    })

    it('should handle different configurations', () => {
      const configs = [
        { taskId: 'task-1', agentType: 'claude-code', workerPort: 8001 },
        { taskId: 'task-2', agentType: 'test-agent', workerPort: 8002 },
        { taskId: 'task-3', agentType: 'custom-agent', workerPort: 8003 },
      ]

      for (const config of configs) {
        const agent = new ProxyAgent({ ...config, logger: mockLogger })
        expect(agent).toBeInstanceOf(ProxyAgent)
      }
    })
  })

  describe('run method', () => {
    it('should initiate agent execution via worker', async () => {
      // Mock successful response
      mockFetch.mockResolvedValue({
        ok: true,
        json: async () => ({ status: 'started' }),
      })

      const generator = proxyAgent.run('test message', { model: 'test-model' })
      const results = []

      for await (const event of generator) {
        results.push(event)
      }

      // Should yield empty array (fire-and-forget pattern)
      expect(results).toEqual([])

      // Verify fetch was called correctly
      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8001/execute-agent',
        expect.objectContaining({
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            agentType: 'test-agent',
            taskId: 'test-task',
            message: 'test message',
            model: 'test-model',
            runOptions: {
              model: 'test-model',
              workspaceDir: '/repo',
            },
          }),
        }),
      )

      // Verify logging
      expect(mockLogger.info).toHaveBeenCalledWith('Executing agent via proxy', {
        workerUrl: 'http://localhost:8001/execute-agent',
      })
      expect(mockLogger.info).toHaveBeenCalledWith('Agent execution started', {
        status: 'started',
      })
    })

    it('should yield empty array (fire-and-forget pattern)', async () => {
      mockFetch.mockResolvedValue({
        ok: true,
        json: async () => ({ status: 'started' }),
      })

      const generator = proxyAgent.run('test message')
      const results = []

      for await (const event of generator) {
        results.push(event)
      }

      expect(results).toEqual([])
    })

    it('should handle worker endpoint errors', async () => {
      // Mock error response
      mockFetch.mockResolvedValue({
        ok: false,
        text: async () => 'Worker not available',
      })

      const generator = proxyAgent.run('test message')

      await expect(async () => {
        for await (const _ of generator) {
          // Should throw before yielding anything
        }
      }).rejects.toThrow('Failed to execute agent in worker: Worker not available')

      // Verify error logging
      expect(mockLogger.error).toHaveBeenCalledWith(
        'Error executing agent via proxy',
        expect.any(Error),
        { workerUrl: 'http://localhost:8001/execute-agent' },
      )
    })

    it('should handle network errors', async () => {
      // Mock network error
      mockFetch.mockRejectedValue(new Error('Network error'))

      const generator = proxyAgent.run('test message')

      await expect(async () => {
        for await (const _ of generator) {
          // Should throw before yielding anything
        }
      }).rejects.toThrow('Network error')

      // Verify error logging
      expect(mockLogger.error).toHaveBeenCalledWith(
        'Error executing agent via proxy',
        expect.any(Error),
        { workerUrl: 'http://localhost:8001/execute-agent' },
      )
    })

    it('should use correct worker port', async () => {
      mockFetch.mockResolvedValue({
        ok: true,
        json: async () => ({ status: 'started' }),
      })

      // Test with different ports
      const agent1 = new ProxyAgent({
        taskId: 'task1',
        agentType: 'agent1',
        workerPort: 9001,
        logger: mockLogger,
      })
      await agent1.run('message').next()

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:9001/execute-agent',
        expect.any(Object),
      )

      const agent2 = new ProxyAgent({
        taskId: 'task2',
        agentType: 'agent2',
        workerPort: 9002,
        logger: mockLogger,
      })
      await agent2.run('message').next()

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:9002/execute-agent',
        expect.any(Object),
      )
    })

    it('should pass run options correctly', async () => {
      mockFetch.mockResolvedValue({
        ok: true,
        json: async () => ({ status: 'started' }),
      })

      const runOptions = {
        model: 'claude-3',
        temperature: 0.7,
        maxTokens: 1000,
      }

      await proxyAgent.run('test message', runOptions).next()

      expect(mockFetch).toHaveBeenCalledWith(
        expect.any(String),
        expect.objectContaining({
          body: JSON.stringify({
            agentType: 'test-agent',
            taskId: 'test-task',
            message: 'test message',
            model: 'claude-3',
            runOptions: {
              ...runOptions,
              workspaceDir: '/repo',
            },
          }),
        }),
      )
    })
  })

  describe('stop method', () => {
    it('should stop agent via POST request', async () => {
      mockFetch.mockResolvedValue({
        ok: true,
        text: async () => 'Agent stopped',
      })

      await proxyAgent.stop()

      // Verify POST request to stop-agent endpoint
      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8001/stop-agent',
        expect.objectContaining({
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            taskId: 'test-task',
          }),
        }),
      )
    })

    it('should handle stop errors gracefully', async () => {
      mockFetch.mockResolvedValue({
        ok: false,
        text: async () => 'Failed to stop agent',
      })

      await expect(proxyAgent.stop()).rejects.toThrow(
        'Failed to stop agent in worker: Failed to stop agent',
      )

      // Verify error logging
      expect(mockLogger.error).toHaveBeenCalledWith(
        'Error stopping remote agent',
        expect.any(Error),
      )
    })

    it('should handle stop network errors', async () => {
      mockFetch.mockRejectedValue(new Error('Connection refused'))

      await expect(proxyAgent.stop()).rejects.toThrow('Connection refused')

      // Verify error logging
      expect(mockLogger.error).toHaveBeenCalledWith(
        'Error stopping remote agent',
        expect.any(Error),
      )
    })
  })

  describe('Error scenarios', () => {
    it('should handle JSON parsing errors', async () => {
      mockFetch.mockResolvedValue({
        ok: true,
        json: async () => {
          throw new Error('Invalid JSON')
        },
      })

      const generator = proxyAgent.run('test message')

      await expect(async () => {
        for await (const _ of generator) {
          // Should throw before yielding anything
        }
      }).rejects.toThrow('Invalid JSON')
    })

    it('should handle timeout scenarios', async () => {
      // Simulate timeout by never resolving
      mockFetch.mockImplementation(
        () =>
          new Promise(() => {
            /* intentionally unresolved */
          }),
      )

      const generator = proxyAgent.run('test message')

      // Create a timeout promise
      const timeoutPromise = new Promise((_, reject) => {
        setTimeout(() => reject(new Error('Timeout')), 100)
      })

      // Race between generator and timeout
      await expect(
        Promise.race([
          (async () => {
            for await (const _ of generator) {
              // Should never reach here
            }
          })(),
          timeoutPromise,
        ]),
      ).rejects.toThrow('Timeout')
    })
  })

  describe('Public API methods', () => {
    it('should return agent type', () => {
      const agent = new ProxyAgent({
        taskId: 'task-123',
        agentType: 'custom-agent',
        workerPort: 8001,
        logger: mockLogger,
      })

      expect(agent.getAgentType()).toBe('custom-agent')
    })

    it('should return session ID when provided', () => {
      const agent = new ProxyAgent({
        taskId: 'task-123',
        agentType: 'test-agent',
        workerPort: 8001,
        sessionId: 'session-456',
        logger: mockLogger,
      })

      expect(agent.getSessionId()).toBe('session-456')
    })

    it('should return undefined session ID when not provided', () => {
      expect(proxyAgent.getSessionId()).toBeUndefined()
    })

    it('should handle getBranchNameAndSummary', async () => {
      mockFetch.mockResolvedValue({
        ok: true,
        json: async () => ({ branch: 'feature/test', summary: 'Test summary' }),
      })

      await proxyAgent.getBranchNameAndSummary('test message')

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8001/agent-branch-summary',
        expect.objectContaining({
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
        }),
      )
    })
  })
})
