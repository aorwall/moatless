import type { RepositoryConfig, TaskResponse } from '@moatless/types'
import { describe, expect, it } from 'vitest'
import { ClaudeService } from '../../services/claude.service'

describe('ClaudeService Integration', () => {
  describe('generateProjectId', () => {
    it('should generate consistent project ID using standard container workspace path', () => {
      const service = new ClaudeService()

      const mockTask: TaskResponse = {
        taskId: 'test-task-id',
        status: 'running',
        iterations: [],
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 1,
        totalCostUsd: 0.01,
        visibility: 'private' as const,
        repositoryId: 'test-repo',
        agentType: 'claude-code',
      }

      const repositoryConfig: RepositoryConfig = {
        kind: 'remote',
        id: 'test-repo',
        name: 'Test Repository',
        remote: {
          url: 'https://github.com/test-owner/test-repo',
          provider: 'github',
        },
        description: 'Test repository',
        visibility: 'private',
        codeExecution: {
          dockerImage: 'ubuntu:latest',
        },
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      const projectId = service.generateProjectId(mockTask, repositoryConfig)

      // Should use the standard CONTAINER_WORKSPACE_PATH (/opt/moatless/repo)
      expect(projectId).toBe('-opt-moatless-repo')
      expect(projectId).toMatch(/^[a-zA-Z0-9-]+$/) // Only alphanumeric and dashes
    })
  })
})
