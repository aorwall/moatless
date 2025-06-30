import type { Task } from '@shared/types/task.js'
import { WorkspaceStatus } from '@shared/types/workspace.js'
import type { FastifyInstance } from 'fastify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../../test/helper.js'

// Mock modules
vi.mock('../../services/agents/claude-code-agent.js', () => {
  return {
    ClaudeCodeAgent: class MockClaudeCodeAgent {
      async *run() {
        yield {
          eventId: 'test-event-1',
          type: 'message',
          taskId: 'test-task',
          timestamp: new Date(),
          isError: false,
          payload: {
            type: 'system',
            subtype: 'init',
            message: 'Mock agent initialized',
          },
        }
      }
      getSessionId() {
        return '12345678-1234-1234-1234-123456789012'
      }
    },
  }
})

describe('Workspace Endpoints', () => {
  let app: FastifyInstance
  const taskId = 'test-task-123'

  const mockTask: Task = {
    taskId,
    environmentId: 'test-env',
    agentType: 'claude-code',
    status: 'running',
    createdAt: new Date(),
    lastMessageAt: new Date(),
    numTurns: 0,
    totalCostUsd: 0,
    iterations: [],
    workspacePath: '/tmp/test-repo',
  }

  beforeEach(async () => {
    app = await build()

    // Mock task service to return our mock task
    vi.spyOn(app.services.task, 'getTask').mockResolvedValue(mockTask)

    // Mock workspace service methods
    vi.spyOn(app.services.workspace, 'getWorkspaceStatus').mockResolvedValue({
      status: WorkspaceStatus.IDLE,
      hasUncommittedChanges: false,
      backgroundTaskCount: 0,
    })
    vi.spyOn(app.services.workspace, 'canListFiles').mockResolvedValue(true)
    vi.spyOn(app.services.workspace, 'canReadWriteFiles').mockResolvedValue(true)
    vi.spyOn(app.services.workspace, 'canSearchFiles').mockResolvedValue(true)
    vi.spyOn(app.services.workspace, 'listFiles').mockResolvedValue(['file1.ts', 'file2.ts'])
    vi.spyOn(app.services.workspace, 'readFile').mockResolvedValue('console.log("test");')
    vi.spyOn(app.services.workspace, 'writeFile').mockResolvedValue(undefined)
    vi.spyOn(app.services.workspace, 'searchFiles').mockResolvedValue({
      results: [{ path: 'file1.ts', absolutePath: '/tmp/test-repo/file1.ts', score: 1 }],
      totalFound: 1,
      query: 'test',
    })
    vi.spyOn(app.services.workspace, 'getGitDiff').mockResolvedValue(
      'diff --git a/file1.ts b/file1.ts',
    )
    vi.spyOn(app.services.workspace, 'runCommand').mockResolvedValue({
      command: 'echo test',
      output: 'test',
      exitCode: 0,
      durationMs: 100,
    })
    vi.spyOn(app.services.workspace, 'startServer').mockResolvedValue({
      port: 3000,
      pid: 12345,
    })
    vi.spyOn(app.services.workspace, 'stopServer').mockResolvedValue(true)
    vi.spyOn(app.services.workspace, 'getServerStatus').mockResolvedValue({
      running: false,
      configured: true,
      uptimeSeconds: 0,
    })
    vi.spyOn(app.services.workspace, 'getServerLogs').mockResolvedValue('Server logs...')
  })

  afterEach(async () => {
    await app.close()
    vi.restoreAllMocks()
  })

  describe('GET /:taskId/workspace-status', () => {
    it('should return workspace status', async () => {
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${taskId}/workspace-status`,
      })

      expect(response.statusCode).toBe(200)
      const status = JSON.parse(response.payload)
      expect(status).toEqual({
        status: 'idle',
        hasUncommittedChanges: false,
        backgroundTaskCount: 0,
      })
    })

    it('should return 404 for non-existent task', async () => {
      // Mock task service to return null for non-existent task
      vi.spyOn(app.services.task, 'getTask').mockResolvedValueOnce(null as any)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/tasks/non-existent/workspace-status',
      })

      expect(response.statusCode).toBe(404)
      const error = JSON.parse(response.payload)
      // Check the actual error format - may be nested under 'error' property
      const errorMessage = error.error?.message || error.message || error.error
      expect(errorMessage).toBe('Task non-existent not found')
    })
  })

  describe('GET /:taskId/files', () => {
    it('should list files in workspace', async () => {
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${taskId}/files`,
      })

      expect(response.statusCode).toBe(200)
      const result = JSON.parse(response.payload)
      expect(result).toEqual({
        files: ['file1.ts', 'file2.ts'],
        totalCount: 2,
      })
    })

    it('should list files with path and recursive params', async () => {
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${taskId}/files`,
        query: {
          path: 'src',
          recursive: 'true',
        },
      })

      expect(response.statusCode).toBe(200)
      expect(app.services.workspace.listFiles).toHaveBeenCalledWith(taskId, 'src', true)
    })

    it('should return error if workspace not setup', async () => {
      vi.mocked(app.services.workspace.getWorkspaceStatus).mockResolvedValueOnce({
        status: WorkspaceStatus.NOT_SETUP,
        hasUncommittedChanges: false,
        backgroundTaskCount: 0,
      })

      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${taskId}/files`,
      })

      expect(response.statusCode).toBe(400)
      const error = JSON.parse(response.payload)
      const errorMessage = error.error?.message || error.message || error.error
      expect(errorMessage).toContain('Workspace not created')
    })
  })

  describe('GET /:taskId/files/:filePath', () => {
    it('should read file content', async () => {
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${taskId}/files/src/test.ts`,
      })

      expect(response.statusCode).toBe(200)
      const result = JSON.parse(response.payload)
      expect(result).toEqual({
        content: 'console.log("test");',
        encoding: 'utf-8',
      })
    })

    it('should handle file not found', async () => {
      vi.mocked(app.services.workspace.readFile).mockRejectedValueOnce(new Error('File not found'))

      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${taskId}/files/non-existent.ts`,
      })

      expect(response.statusCode).toBe(500)
    })
  })

  describe('PUT /:taskId/files/:filePath', () => {
    it('should write file content', async () => {
      const response = await app.inject({
        method: 'PUT',
        url: `/api/v1/tasks/${taskId}/files/src/new.ts`,
        payload: {
          content: 'export const test = 123;',
        },
      })

      expect(response.statusCode).toBe(200)
      const result = JSON.parse(response.payload)
      expect(result).toEqual({ success: true })
      expect(app.services.workspace.writeFile).toHaveBeenCalledWith(
        taskId,
        'src/new.ts',
        'export const test = 123;',
      )
    })
  })

  describe('POST /:taskId/search-files', () => {
    it('should search files by pattern', async () => {
      const response = await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${taskId}/search-files`,
        payload: {
          query: 'console.log',
          limit: 50,
        },
      })

      expect(response.statusCode).toBe(200)
      const result = JSON.parse(response.payload)
      expect(result).toEqual({
        results: [{ path: 'file1.ts', absolutePath: '/tmp/test-repo/file1.ts', score: 1 }],
        totalFound: 1,
        query: 'test',
      })
    })
  })

  describe('GET /:taskId/git-diff', () => {
    it('should return git diff', async () => {
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${taskId}/git-diff`,
      })

      expect(response.statusCode).toBe(200)
      const result = JSON.parse(response.payload)
      expect(result).toEqual({
        diff: 'diff --git a/file1.ts b/file1.ts',
        files: [],
        hasChanges: true,
      })
    })
  })

  describe('POST /:taskId/execute', () => {
    it('should execute command in workspace', async () => {
      const response = await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${taskId}/execute`,
        payload: {
          command: 'echo test',
          timeout: 5000,
        },
      })

      expect(response.statusCode).toBe(200)
      const result = JSON.parse(response.payload)
      expect(result).toEqual({
        command: 'echo test',
        output: 'test',
        exitCode: 0,
        durationMs: 100,
      })
    })

    it('should not allow execution if workspace not idle', async () => {
      vi.mocked(app.services.workspace.getWorkspaceStatus).mockResolvedValueOnce({
        status: WorkspaceStatus.RUNNING,
        hasUncommittedChanges: false,
        backgroundTaskCount: 0,
      })

      const response = await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${taskId}/execute`,
        payload: {
          command: 'echo test',
        },
      })

      expect(response.statusCode).toBe(400)
      const error = JSON.parse(response.payload)
      const errorMessage = error.error?.message || error.message || error.error
      expect(errorMessage).toContain('Cannot execute command: workspace is running')
    })
  })

  describe('Server Management', () => {
    describe('POST /:taskId/server/start', () => {
      it('should start development server', async () => {
        const response = await app.inject({
          method: 'POST',
          url: `/api/v1/tasks/${taskId}/server/start`,
          payload: {
            port: 3000,
          },
        })

        expect(response.statusCode).toBe(200)
        const result = JSON.parse(response.payload)
        expect(result).toMatchObject({
          running: true,
          port: 3000,
          pid: 12345,
        })
      })
    })

    describe('POST /:taskId/server/stop', () => {
      it('should stop development server', async () => {
        const response = await app.inject({
          method: 'POST',
          url: `/api/v1/tasks/${taskId}/server/stop`,
        })

        expect(response.statusCode).toBe(200)
        const result = JSON.parse(response.payload)
        expect(result).toMatchObject({
          stopped: true,
          runtime: 0,
        })
      })
    })

    describe('GET /:taskId/server/status', () => {
      it('should return server status', async () => {
        const response = await app.inject({
          method: 'GET',
          url: `/api/v1/tasks/${taskId}/server/status`,
        })

        expect(response.statusCode).toBe(200)
        const result = JSON.parse(response.payload)
        expect(result).toEqual({
          running: false,
          configured: true,
          uptimeSeconds: 0,
        })
      })
    })

    describe('GET /:taskId/server/logs', () => {
      it('should return server logs', async () => {
        const response = await app.inject({
          method: 'GET',
          url: `/api/v1/tasks/${taskId}/server/logs`,
          query: {
            lines: '50',
          },
        })

        expect(response.statusCode).toBe(200)
        const result = JSON.parse(response.payload)
        expect(result).toMatchObject({
          logs: 'Server logs...',
          lines: 1,
          truncated: false,
        })
        expect(app.services.workspace.getServerLogs).toHaveBeenCalledWith(taskId, '50')
      })
    })
  })
})
