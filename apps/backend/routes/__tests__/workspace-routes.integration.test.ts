import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// Mock git utilities first
vi.mock('../../utils/git-utils.js', () => ({
  isGitWorktree: vi.fn().mockResolvedValue(true),
  isGitRepository: vi.fn().mockResolvedValue(true),
  createGitWorktree: vi.fn().mockResolvedValue(undefined),
  getBranchList: vi.fn().mockResolvedValue(['main', 'develop']),
  syncWithMain: vi.fn().mockResolvedValue({
    success: true,
    conflicts: false,
    message: 'Sync successful',
    commitsBehind: 0,
    commitsAhead: 0,
  }),
  getGitStatus: vi.fn().mockResolvedValue({
    currentBranch: 'main',
    hasUncommittedChanges: false,
    hasConflicts: false,
    status: 'up_to_date',
  }),
}))

vi.mock('../../utils/index.js', async () => {
  const actual = await vi.importActual('../../utils/index.js')
  return {
    ...actual,
    isGitWorktree: vi.fn().mockResolvedValue(true),
    isGitRepository: vi.fn().mockResolvedValue(true),
    createGitWorktree: vi.fn().mockResolvedValue(undefined),
    getBranchList: vi.fn().mockResolvedValue(['main', 'develop']),
  }
})

import type { AppWithServices } from '../../app.js'
import { createApp } from '../../app.js'
import { createMockWorkspace } from '../../test/mocks/workspace.mock.js'

describe('Workspace Routes - Integration Tests', () => {
  let app: AppWithServices
  let mockWorkspace: ReturnType<typeof createMockWorkspace>

  beforeEach(async () => {
    app = await createApp()
    mockWorkspace = createMockWorkspace()

    // Mock workspace service methods
    vi.spyOn(app.services.workspace, 'getOrCreateWorkspace').mockResolvedValue(mockWorkspace as any)
    vi.spyOn(app.services.workspace, 'getWorkspaceStatus').mockResolvedValue({
      // Flattened workspace status structure
      workspaceState: 'exists',
      workspacePath: '/test/workspace',
      workspaceChangeStatus: 'no_changes',

      // Environment info
      environmentType: 'local',
      environmentStatus: 'ready',
      environmentCreatedAt: new Date().toISOString(),

      // Agent info
      agentState: 'idle',

      capabilities: {
        fileOperations: true,
        commandExecution: true,
        serverManagement: true,
      },
      gitStatus: {
        currentBranch: 'main',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'up_to_date',
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: true,
      },

      // Computed states
      isRunning: false,
      workspaceReady: true,
      canExecute: true,
    })
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('GET /api/v1/tasks/:taskId/workspace-status', () => {
    it('should return workspace status', async () => {
      const res = await app.app.request('/api/v1/tasks/test-task/workspace-status')

      expect(res.status).toBe(200)
      const result = await res.json()
      expect(result).toHaveProperty('status')
      expect(result).toHaveProperty('environment')
      expect(result).toHaveProperty('gitStatus')
    })
  })

  describe('GET /api/v1/tasks/:taskId/files', () => {
    it('should list files in workspace', async () => {
      const res = await app.app.request('/api/v1/tasks/test-task/files')

      expect(res.status).toBe(200)
      const result = (await res.json()) as any
      expect(result).toHaveProperty('files')
      expect(result).toHaveProperty('totalCount')
      expect(result.files).toHaveLength(2)
      expect(mockWorkspace.listFiles).toHaveBeenCalledWith('/', false)
    })

    it('should list files with path parameter', async () => {
      const res = await app.app.request('/api/v1/tasks/test-task/files?path=src&recursive=true')

      expect(res.status).toBe(200)
      expect(mockWorkspace.listFiles).toHaveBeenCalledWith('src', true)
    })
  })

  describe('GET /api/v1/tasks/:taskId/files/*', () => {
    it('should read file content', async () => {
      const res = await app.app.request('/api/v1/tasks/test-task/files/test.ts')

      expect(res.status).toBe(200)
      const result = await res.json()
      expect(result).toHaveProperty('content')
      expect(result).toHaveProperty('encoding', 'utf-8')
      expect(mockWorkspace.readFile).toHaveBeenCalledWith('test.ts')
    })
  })

  describe('PUT /api/v1/tasks/:taskId/files/*', () => {
    it('should write file content', async () => {
      const res = await app.app.request('/api/v1/tasks/test-task/files/test.ts', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content: 'console.log("updated");' }),
      })

      expect(res.status).toBe(200)
      const result = await res.json()
      expect(result).toHaveProperty('success', true)
      expect(mockWorkspace.writeFile).toHaveBeenCalledWith('test.ts', 'console.log("updated");')
    })
  })

  describe('POST /api/v1/tasks/:taskId/search-files', () => {
    it('should search files', async () => {
      const res = await app.app.request('/api/v1/tasks/test-task/search-files', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ query: 'test', limit: 50 }),
      })

      expect(res.status).toBe(200)
      const result = await res.json()
      expect(result).toHaveProperty('results')
      expect(result).toHaveProperty('totalFound')
      expect(mockWorkspace.searchFiles).toHaveBeenCalledWith({ query: 'test', limit: 50 })
    })
  })

  describe('GET /api/v1/tasks/:taskId/git-diff', () => {
    it('should get git diff', async () => {
      const res = await app.app.request('/api/v1/tasks/test-task/git-diff')

      expect(res.status).toBe(200)
      const result = await res.json()
      expect(result).toHaveProperty('diff')
      expect(result).toHaveProperty('files')
      expect(result).toHaveProperty('hasChanges')
      expect(mockWorkspace.getGitDiff).toHaveBeenCalled()
    })
  })

  describe('GET /api/v1/tasks/:taskId/file-tree-with-changes', () => {
    it('should get file tree with changes', async () => {
      const res = await app.app.request('/api/v1/tasks/test-task/file-tree-with-changes')

      expect(res.status).toBe(200)
      const result = await res.json()
      expect(result).toHaveProperty('tree')
      expect(result).toHaveProperty('gitSummary')
      expect(mockWorkspace.getFileTreeWithChanges).toHaveBeenCalledWith('.')
    })
  })

  describe('POST /api/v1/tasks/:taskId/sync-with-main', () => {
    it('should sync workspace with main branch', async () => {
      const res = await app.app.request('/api/v1/tasks/test-task/sync-with-main', {
        method: 'POST',
      })

      expect(res.status).toBe(200)
      const result = await res.json()
      expect(result).toHaveProperty('success', true)
      expect(result).toHaveProperty('conflicts', false)
      expect(mockWorkspace.syncWithMain).toHaveBeenCalledWith(undefined)
    })
  })

  describe('POST /api/v1/tasks/:taskId/execute', () => {
    it('should execute command in workspace', async () => {
      const res = await app.app.request('/api/v1/tasks/test-task/execute', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ command: 'ls', timeout: 5000 }),
      })

      expect(res.status).toBe(200)
      const result = await res.json()
      expect(result).toHaveProperty('command')
      expect(result).toHaveProperty('output')
      expect(result).toHaveProperty('exitCode')
      expect(mockWorkspace.runCommand).toHaveBeenCalledWith('ls', 5000)
    })
  })
})
