import * as os from 'node:os'
import * as path from 'node:path'
import type { RepositoryConfig, TaskResponse } from '@moatless/types'
import { beforeAll, describe, expect, it } from 'vitest'
import { ClaudeService } from '../claude.service'
import { fileExists } from '../claude-utils'

describe('ClaudeService Integration - Real Session File', () => {
  let service: ClaudeService
  let realSessionFilePath: string
  let sessionExists: boolean

  const sessionId = '58a94d13-647a-4d38-b628-fa057f26ec91'
  const projectPath = '-Users-albert-repos-moatless-moatless-swarm-ui'

  beforeAll(async () => {
    // @ts-expect-error - container not available in test context
    // biome-ignore lint/correctness/noUndeclaredVariables: using global DI container in integration test
    const settingsService = container.resolve(SettingsService)
    service = new ClaudeService(settingsService)

    // Build the path to the real session file
    const homeDir = os.homedir()
    realSessionFilePath = path.join(
      homeDir,
      '.claude',
      'projects',
      projectPath,
      `${sessionId}.jsonl`,
    )

    // Check if the session file exists
    sessionExists = await fileExists(realSessionFilePath)
  })

  describe('getSessionUsage with real data', () => {
    it('should read and parse real session file correctly', async () => {
      if (!sessionExists) {
        console.log(`Skipping test - session file not found: ${realSessionFilePath}`)
        return
      }

      const result = await service.getSessionUsage(sessionId)

      expect(result).toBeDefined()
      expect(result.sessionId).toBe(sessionId)
      expect(result.cost).toBeGreaterThanOrEqual(0)
      expect(result.tokens).toBeDefined()
      expect(result.tokens.input_tokens).toBeGreaterThanOrEqual(0)
      expect(result.tokens.output_tokens).toBeGreaterThanOrEqual(0)
      expect(result.tokens.total_tokens).toBeGreaterThanOrEqual(0)
      expect(result.messageCount).toBeGreaterThan(0)
      expect(result.modelDistribution).toBeDefined()

      // Verify token totals are consistent
      expect(result.tokens.total_tokens).toBe(
        result.tokens.input_tokens + result.tokens.output_tokens,
      )

      // Verify model distribution percentages (only check if there are models)
      const distributionSum = Object.values(result.modelDistribution).reduce(
        (sum, percentage) => sum + percentage,
        0,
      )

      if (Object.keys(result.modelDistribution).length > 0) {
        // If there are models, distribution should add up to 100 (or close to it due to rounding)
        expect(distributionSum).toBeGreaterThanOrEqual(99)
        expect(distributionSum).toBeLessThanOrEqual(101)
      } else {
        // If no models, distribution sum should be 0
        expect(distributionSum).toBe(0)
      }

      console.log('Session usage results:', {
        sessionId: result.sessionId,
        cost: result.cost,
        messageCount: result.messageCount,
        totalTokens: result.tokens.total_tokens,
        modelDistribution: result.modelDistribution,
      })
    })

    it('should handle session with specific repository config', async () => {
      if (!sessionExists) {
        console.log(`Skipping test - session file not found: ${realSessionFilePath}`)
        return
      }

      // Repository config no longer needed since getSessionUsage doesn't use it
      const result = await service.getSessionUsage(sessionId)

      expect(result).toBeDefined()
      expect(result.sessionId).toBe(sessionId)
      expect(result.messageCount).toBeGreaterThan(0)
      expect(result.cost).toBeGreaterThanOrEqual(0)
    })
  })

  describe('readClaudeMessages with real data', () => {
    it('should read messages from real session file using task context', async () => {
      if (!sessionExists) {
        console.log(`Skipping test - session file not found: ${realSessionFilePath}`)
        return
      }

      // Create a mock task with the session ID
      const mockTask: TaskResponse = {
        taskId: 'test-task-id',
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: sessionId,
            initialMessage: 'test message',
            startTime: new Date(),
            status: 'active',
            messageCount: 1,
            costUsd: 0.01,
          },
        ],
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 1,
        totalCostUsd: 0.01,

        repositoryId: 'test-repo',
        agentType: 'claude-code',
      }

      // Repository config no longer needed since readClaudeMessages only takes agentSessionId
      // Test reading messages for each iteration's session
      const allMessages = []
      for (const iteration of mockTask.iterations) {
        if (iteration.agentSessionId) {
          const messages = await service.readClaudeMessages(iteration.agentSessionId)
          allMessages.push(...messages)
        }
      }

      expect(allMessages).toBeDefined()
      expect(Array.isArray(allMessages)).toBe(true)
      expect(allMessages.length).toBeGreaterThan(0)

      // Verify structure of the first message
      const firstMessage = allMessages[0]
      expect(firstMessage).toBeDefined()
      if (!firstMessage) throw new Error('Expected first message')
      expect(firstMessage.type).toBeDefined()

      // Count different message types
      const userMessages = allMessages.filter((msg) => msg.type === 'user')
      const assistantMessages = allMessages.filter((msg) => msg.type === 'assistant')

      expect(userMessages.length).toBeGreaterThan(0)
      expect(assistantMessages.length).toBeGreaterThan(0)

      console.log('Messages analysis:', {
        totalMessages: allMessages.length,
        userMessages: userMessages.length,
        assistantMessages: assistantMessages.length,
        messageTypes: [...new Set(allMessages.map((msg) => msg.type))],
      })
    })
  })

  describe('generateProjectId', () => {
    it('should generate consistent project ID for docker repositories', () => {
      const mockTask: TaskResponse = {
        taskId: 'test-task-id',
        status: 'running',
        workspacePath: '/some/workspace/path',
        iterations: [],
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 1,
        totalCostUsd: 0.01,

        repositoryId: 'test-repo',
        agentType: 'claude-code',
      }

      const repositoryConfig: RepositoryConfig = {
        kind: 'local',
        id: 'test-repo',
        name: 'Test Repository',
        path: '/Users/albert/repos/moatless/moatless-swarm-ui',
        useGitWorktrees: false,
        description: 'Test repository',
        codeExecution: {
          dockerImage: 'ubuntu:latest',
        },
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      const projectId = service.generateProjectId(mockTask, repositoryConfig)

      expect(projectId).toBe('-Users-albert-repos-moatless-moatless-swarm-ui')
      expect(projectId).toMatch(/^[a-zA-Z0-9-]+$/) // Only alphanumeric and dashes
    })
  })

  describe('getClaudeMessagesFilePath', () => {
    it('should generate correct file path for session', async () => {
      const mockTask: TaskResponse = {
        taskId: 'test-task-id',
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [],
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 1,
        totalCostUsd: 0.01,

        repositoryId: 'test-repo',
        agentType: 'claude-code',
      }

      const repositoryConfig: RepositoryConfig = {
        kind: 'local',
        id: 'test-repo',
        name: 'Test Repository',
        path: '/Users/albert/repos/moatless/moatless-swarm-ui',
        useGitWorktrees: false,
        description: 'Test repository',
        codeExecution: {
          dockerImage: 'ubuntu:latest',
        },
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      // Iteration details not needed for this test

      const projectId = service.generateProjectId(mockTask, repositoryConfig)
      const filePath = await service.getClaudeMessagesFilePath(sessionId, projectId)

      expect(filePath).toBeDefined()
      expect(filePath).toContain(sessionId)
      expect(filePath).toContain('.jsonl')
      expect(filePath).toBe(realSessionFilePath)
    })

    it('should generate correct file path with custom project ID', async () => {
      const customSessionId = 'custom-session-123'
      const customProjectId = 'my-custom-project'

      const filePath = await service.getClaudeMessagesFilePath(customSessionId, customProjectId)

      expect(filePath).toBeDefined()
      expect(filePath).toContain(customSessionId)
      expect(filePath).toContain(customProjectId)
      expect(filePath.endsWith(`${customSessionId}.jsonl`)).toBe(true)
      expect(filePath).toContain(path.join(customProjectId, `${customSessionId}.jsonl`))
    })
  })

  describe('error handling', () => {
    it('should handle non-existent session gracefully', async () => {
      const nonExistentSessionId = 'non-existent-session-id'

      const result = await service.getSessionUsage(nonExistentSessionId)

      expect(result).toBeDefined()
      expect(result.sessionId).toBe(nonExistentSessionId)
      expect(result.cost).toBe(0)
      expect(result.tokens.total_tokens).toBe(0)
      expect(result.messageCount).toBe(0)
      expect(Object.keys(result.modelDistribution)).toHaveLength(0)
    })

    it('should handle invalid project path gracefully', async () => {
      // Task mock not needed for this test

      // Test reading messages for the invalid session
      const messages = await service.readClaudeMessages('invalid-session')

      expect(messages).toBeDefined()
      expect(Array.isArray(messages)).toBe(true)
      expect(messages.length).toBe(0) // Should return empty array for non-existent files
    })
  })
})
