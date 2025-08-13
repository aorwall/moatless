import { describe, expect, it } from 'vitest'
import { MessageSchema } from '../message.schemas.js'
import { RepositoryConfigSchema } from '../repository.schemas.js'
import { TaskResponseSchema } from '../task.schemas.js'

describe('Date Handling Consistency', () => {
  it('should accept ISO string dates in TaskResponseSchema', () => {
    const taskData = {
      taskId: 'test-123',
      createdAt: '2024-01-15T10:30:00Z',
      lastMessageAt: '2024-01-15T10:35:00Z',
      agentType: 'test-agent',
      repositoryId: 'repo-123',
      status: 'open',
      numTurns: 5,
      totalCostUsd: 0.05,
      iterations: [
        {
          iterationNumber: 1,
          startTime: '2024-01-15T10:30:00Z',
          endTime: '2024-01-15T10:32:00Z',
          status: 'completed',
          messageCount: 3,
          costUsd: 0.03,
        },
      ],
    }

    const result = TaskResponseSchema.parse(taskData)

    // Should convert string dates to Date objects
    expect(result.createdAt).toBeInstanceOf(Date)
    expect(result.lastMessageAt).toBeInstanceOf(Date)
    expect(result.iterations[0].startTime).toBeInstanceOf(Date)
    expect(result.iterations[0].endTime).toBeInstanceOf(Date)
  })

  it('should accept Date objects in RepositoryConfigSchema', () => {
    const now = new Date()
    const repoData = {
      id: 'repo-123',
      name: 'test-repo',
      path: '/path/to/repo',
      codeExecution: {
        type: 'local' as const,
      },
      createdAt: now,
      updatedAt: now,
    }

    const result = RepositoryConfigSchema.parse(repoData)

    // Should pass through Date objects unchanged
    expect(result.createdAt).toBe(now)
    expect(result.updatedAt).toBe(now)
  })

  it('should handle message timestamps correctly', () => {
    const messageData = {
      id: 'msg-123',
      role: 'user' as const,
      content: 'Hello',
      timestamp: '2024-01-15T10:30:00Z',
    }

    const result = MessageSchema.parse(messageData)

    // Should convert string to Date
    expect(result.timestamp).toBeInstanceOf(Date)
    expect(result.timestamp.toISOString()).toBe('2024-01-15T10:30:00.000Z')
  })

  it('should handle failed iterations with result', () => {
    const taskData = {
      taskId: 'test-123',
      createdAt: new Date(),
      lastMessageAt: new Date(),
      agentType: 'test-agent',
      repositoryId: 'repo-123',
      status: 'error',
      numTurns: 1,
      totalCostUsd: 0.01,
      iterations: [
        {
          iterationNumber: 1,
          startTime: new Date(),
          endTime: new Date(),
          status: 'failed',
          messageCount: 1,
          costUsd: 0.01,
          result: 'Test error occurred',
        },
      ],
    }

    const result = TaskResponseSchema.parse(taskData)

    // Should handle failed iteration with result
    expect(result.iterations[0].status).toBe('failed')
    expect(result.iterations[0].result).toBe('Test error occurred')
    expect(result.iterations[0].endTime).toBeInstanceOf(Date)
  })
})
