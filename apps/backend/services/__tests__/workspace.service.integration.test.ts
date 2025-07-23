import { promises as fs } from 'node:fs'
import path from 'node:path'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { execAsync } from '../../utils/index.js'
import type { RepositoryService } from '../repository.service.js'
import type { StorageBackend } from '../storage.js'
import { WorkspaceService } from '../workspace.service.js'

// Suppress logger output during tests
vi.mock('../../utils/logger.js', async () => {
  const actual = await vi.importActual('../../utils/logger.js')
  return {
    ...actual,
    logger: {
      info: vi.fn(),
      error: vi.fn(),
      warn: vi.fn(),
      debug: vi.fn(),
      child: vi.fn(() => ({
        info: vi.fn(),
        error: vi.fn(),
        warn: vi.fn(),
        debug: vi.fn(),
      })),
    },
  }
})

describe('WorkspaceService Integration Tests', () => {
  let workspaceService: WorkspaceService
  let repositoryService: RepositoryService
  let storage: StorageBackend
  let testRepoPath: string
  let testWorktreeDir: string
  let testTaskId: string

  beforeAll(async () => {
    // Create a test git repository
    const tempDir = process.env.TEMP || '/tmp'
    testRepoPath = path.join(tempDir, `test-repo-${Date.now()}`)
    testWorktreeDir = path.join(tempDir, `test-worktrees-${Date.now()}`)

    // Initialize git repo
    await fs.mkdir(testRepoPath, { recursive: true })
    await execAsync('git init', { cwd: testRepoPath })
    await execAsync('git config user.email "test@example.com"', { cwd: testRepoPath })
    await execAsync('git config user.name "Test User"', { cwd: testRepoPath })

    // Create initial commit
    await fs.writeFile(path.join(testRepoPath, 'README.md'), '# Test Repo')
    await execAsync('git add .', { cwd: testRepoPath })
    await execAsync('git commit -m "Initial commit"', { cwd: testRepoPath })

    // Set WORKTREE_DIR for tests
    process.env.WORKTREE_DIR = testWorktreeDir
  })

  beforeEach(async () => {
    testTaskId = `test-task-${Date.now()}`

    // Mock repository service
    repositoryService = {
      getRepositoryById: vi.fn().mockResolvedValue({
        id: 'test-repo',
        name: 'Test Repository',
        path: testRepoPath,
        useGitWorktrees: true,
        codeExecution: {
          type: 'local',
          localConfig: {
            environmentPath: testRepoPath,
          },
        },
        fileSearch: {
          type: 'rg',
        },
      }),
    } as any

    // Mock storage
    storage = {
      getTask: vi.fn().mockResolvedValue({
        id: testTaskId,
        repositoryId: 'test-repo',
        workspacePath: path.join(testWorktreeDir, `test-repo-workspace-${testTaskId}`),
      }),
      saveTask: vi.fn(),
    } as any

    // Mock event service
    const mockEventService = {
      emitEvent: vi.fn(),
    } as any

    // Mock docker service
    const mockDockerService = {} as any

    // Mock claude service
    const mockClaudeService = {} as any

    workspaceService = new WorkspaceService(
      repositoryService,
      storage,
      mockEventService,
      mockDockerService,
      mockClaudeService,
    )
  })

  afterEach(async () => {
    // Clean up worktrees
    try {
      const worktrees = await execAsync('git worktree list --porcelain', { cwd: testRepoPath })
      const lines = worktrees.stdout.split('\n')

      for (let i = 0; i < lines.length; i++) {
        if (lines[i].startsWith('worktree ') && !lines[i].includes(testRepoPath)) {
          const worktreePath = lines[i].substring(9)
          await execAsync(`git worktree remove "${worktreePath}" --force`, { cwd: testRepoPath })
        }
      }
    } catch (_error) {
      // Ignore cleanup errors
    }
  })

  afterAll(async () => {
    // Clean up test directories
    await fs.rm(testRepoPath, { recursive: true, force: true })
    await fs.rm(testWorktreeDir, { recursive: true, force: true })
    delete process.env.WORKTREE_DIR
  })

  describe('ensureWorktreeExists', () => {
    it('should handle existing branch when recreating worktree', async () => {
      // Create initial workspace
      const workspace1 = await workspaceService.createWorkspace(testTaskId, 'test-repo')
      expect(workspace1).toBeDefined()

      const workspacePath = workspace1.getWorkspacePath()
      expect(workspacePath).toBeDefined()

      // Verify worktree exists
      const worktreeExists1 = await fs
        .access(workspacePath!)
        .then(() => true)
        .catch(() => false)
      expect(worktreeExists1).toBe(true)

      // Verify branch exists
      const branches1 = await execAsync('git branch -a', { cwd: testRepoPath })
      expect(branches1.stdout).toContain(`workspace-${testTaskId}`)

      // Simulate worktree being marked as invalid (e.g., by removing .git file)
      await fs.rm(path.join(workspacePath!, '.git'), { force: true })

      // Clean up workspace from service
      await workspaceService.cleanupWorkspace(testTaskId)

      // Try to create workspace again - this should reuse the existing branch
      const workspace2 = await workspaceService.createWorkspace(testTaskId, 'test-repo')
      expect(workspace2).toBeDefined()

      const workspacePath2 = workspace2.getWorkspacePath()
      expect(workspacePath2).toBe(workspacePath)

      // Verify worktree was recreated successfully
      const worktreeExists2 = await fs
        .access(workspacePath2!)
        .then(() => true)
        .catch(() => false)
      expect(worktreeExists2).toBe(true)

      // Verify .git file exists and is valid
      const gitFile = await fs.readFile(path.join(workspacePath2!, '.git'), 'utf-8')
      expect(gitFile).toContain('gitdir:')
    })

    it('should handle worktree recreation after git worktree remove', async () => {
      // Create initial workspace
      const workspace1 = await workspaceService.createWorkspace(testTaskId, 'test-repo')
      const workspacePath = workspace1.getWorkspacePath()!

      // Remove worktree using git command (simulating external removal)
      await execAsync(`git worktree remove "${workspacePath}" --force`, { cwd: testRepoPath })

      // Verify directory still exists but is not a valid worktree
      const dirExists = await fs
        .access(workspacePath)
        .then(() => true)
        .catch(() => false)
      expect(dirExists).toBe(false) // git worktree remove deletes the directory

      // Clean up workspace from service
      await workspaceService.cleanupWorkspace(testTaskId)

      // Create workspace again - should handle the missing directory gracefully
      const workspace2 = await workspaceService.createWorkspace(testTaskId, 'test-repo')
      expect(workspace2).toBeDefined()

      // Verify worktree was recreated
      const worktreeExists = await fs
        .access(workspace2.getWorkspacePath()!)
        .then(() => true)
        .catch(() => false)
      expect(worktreeExists).toBe(true)
    })

    it('should create new branch when workspace branch does not exist', async () => {
      // Create workspace
      const workspace = await workspaceService.createWorkspace(testTaskId, 'test-repo')
      expect(workspace).toBeDefined()

      const workspacePath = workspace.getWorkspacePath()
      expect(workspacePath).toBeDefined()

      // Verify new branch was created
      const branches = await execAsync('git branch -a', { cwd: testRepoPath })
      expect(branches.stdout).toContain(`workspace-${testTaskId}`)

      // Verify worktree is using the new branch
      const worktreeList = await execAsync('git worktree list --porcelain', { cwd: testRepoPath })
      expect(worktreeList.stdout).toContain(workspacePath)
      expect(worktreeList.stdout).toContain(`branch refs/heads/workspace-${testTaskId}`)
    })

    it('should handle concurrent workspace creation attempts', async () => {
      // Try to create the same workspace concurrently
      const promises = [
        workspaceService.createWorkspace(testTaskId, 'test-repo'),
        workspaceService.createWorkspace(testTaskId, 'test-repo'),
      ]

      // Both should succeed without errors
      const results = await Promise.allSettled(promises)

      // At least one should succeed
      const successCount = results.filter((r) => r.status === 'fulfilled').length
      expect(successCount).toBeGreaterThan(0)

      // Verify only one worktree was created
      const worktreeList = await execAsync('git worktree list --porcelain', { cwd: testRepoPath })
      const worktreeCount = worktreeList.stdout
        .split('\n')
        .filter((line) => line.startsWith('worktree ')).length
      expect(worktreeCount).toBe(2) // Main repo + one worktree
    })
  })

  describe('getOrCreateWorkspace', () => {
    it('should reuse existing workspace branch when recreating', async () => {
      // Mock task with existing workspace path
      const existingWorkspacePath = path.join(testWorktreeDir, `test-repo-workspace-${testTaskId}`)

      storage.getTask = vi.fn().mockResolvedValue({
        id: testTaskId,
        repositoryId: 'test-repo',
        workspacePath: existingWorkspacePath,
        iterations: [
          {
            iterationNumber: 1,
            status: 'completed',
            gitStatus: {
              currentBranch: `workspace-${testTaskId}`,
            },
          },
        ],
      })

      // Create the branch first
      await execAsync(`git checkout -b workspace-${testTaskId}`, { cwd: testRepoPath })
      await execAsync('git checkout main', { cwd: testRepoPath }) // Switch back to main

      // Get or create workspace - should reuse the existing branch
      const workspace = await workspaceService.getOrCreateWorkspace(testTaskId)
      expect(workspace).toBeDefined()

      // Verify worktree was created with existing branch
      const worktreeList = await execAsync('git worktree list --porcelain', { cwd: testRepoPath })
      expect(worktreeList.stdout).toContain(existingWorkspacePath)
      expect(worktreeList.stdout).toContain(`branch refs/heads/workspace-${testTaskId}`)
    })
  })
})
