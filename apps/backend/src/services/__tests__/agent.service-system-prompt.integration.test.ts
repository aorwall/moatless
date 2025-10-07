import { randomUUID } from 'node:crypto'
import type { AgentEvent } from '@moatless/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { AgentService } from '../agent.service.js'

describe('AgentService - System Prompt Integration', () => {
  let agentService: AgentService
  let mockWorkspaceService: any
  let mockRepositoryService: any
  let mockStorage: any
  let mockMessageService: any
  let mockWorkspace: any
  let mockEnvironment: any

  beforeEach(() => {
    vi.clearAllMocks()

    // Mock environment that tracks runAgent calls
    mockEnvironment = {
      runAgent: vi.fn().mockImplementation(async function* (
        _agentType: string,
        _message: string,
        options?: any,
      ) {
        // Yield a test event to simulate agent execution
        yield {
          taskId: 'test-task',
          type: 'agent',
          subType: 'message',
          agentType: 'claude-code',
          iterationId: options?.iterationNumber || 1,
          payload: {
            text: 'Test response',
            agentSessionId: 'test-session-id',
          },
          eventId: randomUUID(),
          timestamp: new Date(),
          isError: false,
        } as AgentEvent

        // Yield completion event
        yield {
          taskId: 'test-task',
          type: 'agent',
          subType: 'completed',
          agentType: 'claude-code',
          iterationId: options?.iterationNumber || 1,
          payload: {
            subtype: 'success',
            total_cost_usd: 0.05,
            agentSessionId: 'test-session-id',
          },
          eventId: randomUUID(),
          timestamp: new Date(),
          isError: false,
        } as AgentEvent
      }),
      getAgentStatus: vi.fn().mockReturnValue({
        state: 'running',
        sessionId: 'test-session-id',
      }),
      stopAgent: vi.fn(),
    }

    // Mock workspace
    mockWorkspace = {
      getStatus: vi.fn().mockReturnValue('ready'),
      getEnvironment: vi.fn().mockReturnValue(mockEnvironment),
      getWorkspacePath: vi.fn().mockReturnValue('/test/workspace'),
      isWorkspacePathValid: vi.fn().mockResolvedValue(true),
    }

    // Mock workspace service
    mockWorkspaceService = {
      getOrCreateWorkspace: vi.fn().mockResolvedValue(mockWorkspace),
      getWorkspace: vi.fn().mockResolvedValue(mockWorkspace),
    }

    // Mock repository service
    mockRepositoryService = {
      getRepositoryById: vi.fn().mockResolvedValue({
        id: 'test-repo',
        name: 'Test Repository',
        systemPrompt: 'You are a test assistant with custom system prompt',
        previewConfig: {
          serverStartCommands: ['npm run dev'],
        },
        codeExecution: {
          setupCommands: ['npm install'],
        },
      }),
    }

    // Mock storage
    mockStorage = {
      getTask: vi.fn().mockResolvedValue({
        id: 'test-task',
        name: 'Test Task',
        repositoryId: 'test-repo',
        status: 'running',
      }),
    }

    // Mock message service
    mockMessageService = {
      saveUserMessage: vi.fn().mockResolvedValue(undefined),
      saveAgentMessage: vi.fn().mockResolvedValue(undefined),
    }

    agentService = new AgentService(
      mockWorkspaceService,
      mockRepositoryService,
      mockStorage,
      mockMessageService,
    )
  })

  describe('System Prompt Flow', () => {
    it('should pass system prompt from repository through to environment.runAgent', async () => {
      const iteration = {
        iterationNumber: 1,
        status: 'active' as const,
        agentSessionId: 'test-session',
        startTime: new Date(),
        initialMessage: 'Build a hello world app',
        messageCount: 0,
        costUsd: 0,
      }

      // In the real flow, task.service fetches the repository and passes systemPrompt to agent.service
      const repositorySystemPrompt = 'You are a test assistant with custom system prompt'

      const events: AgentEvent[] = []
      for await (const event of agentService.executeAgent({
        taskId: 'test-task',
        message: 'Build a hello world app',
        iteration,
        agentType: 'claude-code',
        model: 'claude-sonnet-4-20250514',
        systemPrompt: repositorySystemPrompt,
      })) {
        events.push(event as AgentEvent)
      }

      // Verify environment.runAgent was called with appendSystemPrompt
      expect(mockEnvironment.runAgent).toHaveBeenCalledWith(
        'claude-code',
        'Build a hello world app',
        expect.objectContaining({
          workspaceDir: '/test/workspace',
          iterationNumber: 1,
          model: 'claude-sonnet-4-20250514',
          hasPreviewServer: true,
          appendSystemPrompt: expect.stringContaining(
            'You are a test assistant with custom system prompt',
          ),
        }),
      )

      // Verify the appendSystemPrompt contains all expected parts
      const callArgs = mockEnvironment.runAgent.mock.calls[0][2]
      expect(callArgs.appendSystemPrompt).toContain(
        'You are a test assistant with custom system prompt',
      )
      expect(callArgs.appendSystemPrompt).toContain('**Interface Layout**')
      expect(callArgs.appendSystemPrompt).toContain('**CRITICAL - Preview Server Management**')
      expect(callArgs.appendSystemPrompt).toContain('**Repository Setup Commands**')
    })

    it('should build system prompt without repository system prompt when not provided', async () => {
      // Mock repository without systemPrompt
      mockRepositoryService.getRepositoryById.mockResolvedValue({
        id: 'test-repo',
        name: 'Test Repository',
        previewConfig: {
          serverStartCommands: ['npm run dev'],
        },
        codeExecution: {
          setupCommands: ['npm install'],
        },
      })

      const iteration = {
        iterationNumber: 1,
        status: 'active' as const,
        agentSessionId: 'test-session',
        startTime: new Date(),
        initialMessage: 'Build a hello world app',
        messageCount: 0,
        costUsd: 0,
      }

      const events: AgentEvent[] = []
      for await (const event of agentService.executeAgent({
        taskId: 'test-task',
        message: 'Build a hello world app',
        iteration,
        agentType: 'claude-code',
      })) {
        events.push(event as AgentEvent)
      }

      // Verify environment.runAgent was called with appendSystemPrompt (but without custom prompt)
      expect(mockEnvironment.runAgent).toHaveBeenCalledWith(
        'claude-code',
        'Build a hello world app',
        expect.objectContaining({
          workspaceDir: '/test/workspace',
          appendSystemPrompt: expect.any(String),
        }),
      )

      // Verify the appendSystemPrompt does NOT contain custom repository prompt
      const callArgs = mockEnvironment.runAgent.mock.calls[0][2]
      expect(callArgs.appendSystemPrompt).not.toContain(
        'You are a test assistant with custom system prompt',
      )
      // But it should still contain other parts
      expect(callArgs.appendSystemPrompt).toContain('**Interface Layout**')
    })

    it('should include task metadata prompt for first iteration when task has no name', async () => {
      // Mock task without name
      mockStorage.getTask.mockResolvedValue({
        id: 'test-task',
        repositoryId: 'test-repo',
        status: 'running',
      })

      const iteration = {
        iterationNumber: 1,
        status: 'active' as const,
        agentSessionId: 'test-session',
        startTime: new Date(),
        initialMessage: 'Build a hello world app',
        messageCount: 0,
        costUsd: 0,
      }

      const events: AgentEvent[] = []
      for await (const event of agentService.executeAgent({
        taskId: 'test-task',
        message: 'Build a hello world app',
        iteration,
        agentType: 'claude-code',
      })) {
        events.push(event as AgentEvent)
      }

      // Verify appendSystemPrompt includes task metadata prompt
      const callArgs = mockEnvironment.runAgent.mock.calls[0][2]
      expect(callArgs.appendSystemPrompt).toContain('mcp__moatless-tools__updateTaskMetadata')
    })

    it('should not include task metadata prompt for subsequent iterations', async () => {
      const iteration = {
        iterationNumber: 2,
        status: 'active' as const,
        agentSessionId: 'test-session',
        startTime: new Date(),
        initialMessage: 'Continue working',
        messageCount: 0,
        costUsd: 0,
      }

      const events: AgentEvent[] = []
      for await (const event of agentService.executeAgent({
        taskId: 'test-task',
        message: 'Continue working',
        iteration,
        agentType: 'claude-code',
      })) {
        events.push(event as AgentEvent)
      }

      // Verify appendSystemPrompt does NOT include task metadata prompt
      const callArgs = mockEnvironment.runAgent.mock.calls[0][2]
      expect(callArgs.appendSystemPrompt).not.toContain('mcp__moatless-tools__updateTaskMetadata')
    })

    it('should not include preview server prompts when repository has no preview config', async () => {
      // Mock repository without preview config
      mockRepositoryService.getRepositoryById.mockResolvedValue({
        id: 'test-repo',
        name: 'Test Repository',
        systemPrompt: 'Custom prompt',
        codeExecution: {
          setupCommands: ['npm install'],
        },
      })

      const iteration = {
        iterationNumber: 1,
        status: 'active' as const,
        agentSessionId: 'test-session',
        startTime: new Date(),
        initialMessage: 'Build a CLI app',
        messageCount: 0,
        costUsd: 0,
      }

      const customSystemPrompt = 'Custom prompt'

      const events: AgentEvent[] = []
      for await (const event of agentService.executeAgent({
        taskId: 'test-task',
        message: 'Build a CLI app',
        iteration,
        agentType: 'claude-code',
        systemPrompt: customSystemPrompt,
      })) {
        events.push(event as AgentEvent)
      }

      // Verify appendSystemPrompt does NOT include preview server content
      const callArgs = mockEnvironment.runAgent.mock.calls[0][2]
      expect(callArgs.appendSystemPrompt).not.toContain('**Interface Layout**')
      expect(callArgs.appendSystemPrompt).not.toContain('**CRITICAL - Preview Server Management**')
      // But should still have custom prompt
      expect(callArgs.appendSystemPrompt).toContain('Custom prompt')
    })

    it('should handle explicit systemPrompt parameter override', async () => {
      const iteration = {
        iterationNumber: 1,
        status: 'active' as const,
        agentSessionId: 'test-session',
        startTime: new Date(),
        initialMessage: 'Build an app',
        messageCount: 0,
        costUsd: 0,
      }

      const explicitSystemPrompt = 'This is an explicit override system prompt'

      const events: AgentEvent[] = []
      for await (const event of agentService.executeAgent({
        taskId: 'test-task',
        message: 'Build an app',
        iteration,
        agentType: 'claude-code',
        systemPrompt: explicitSystemPrompt,
      })) {
        events.push(event as AgentEvent)
      }

      // Verify the explicit system prompt is included
      const callArgs = mockEnvironment.runAgent.mock.calls[0][2]
      expect(callArgs.appendSystemPrompt).toContain(explicitSystemPrompt)
    })
  })
})
