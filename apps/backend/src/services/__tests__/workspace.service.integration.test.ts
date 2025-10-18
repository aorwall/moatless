import { promises as fs } from 'node:fs'
import path from 'node:path'
import { PreviewServerRegistry } from '@moatless/preview-server'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StorageBackend } from '../../storage/index.js'
import { execAsync } from '../../utils/index.js'
import type { RepositoryService } from '../repository.service.js'
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

describe('WorkspaceService Integration Tests (clone mode)', () => {
  let workspaceService: WorkspaceService
  let repositoryService: RepositoryService
  let storage: StorageBackend
  let testRepoPath: string
  let cloneDir: string
  let previewRegistry: PreviewServerRegistry
  let currentTaskId: string

  const createRepositoryConfig = () => ({
    id: 'test-repo',
    name: 'Test Repository',
    path: testRepoPath,
    remoteUrl: testRepoPath,
    cloneOnNewTask: true,
    useGitWorktrees: false,
    codeExecution: {
      type: 'local' as const,
      localConfig: {
        environmentPath: testRepoPath,
      },
    },
    fileSearch: {
      type: 'rg' as const,
    },
  })

  const getExpectedClonePath = (taskId: string) => {
    const repoLastDirName = path.basename(testRepoPath)
    return path.join(cloneDir, `${repoLastDirName}-clone-${taskId}`)
  }

  beforeAll(async () => {
    const tempDir = process.env['TEMP'] || '/tmp'
    testRepoPath = path.join(tempDir, `workspace-service-remote-${Date.now()}`)
    cloneDir = path.join(tempDir, `workspace-service-clones-${Date.now()}`)

    await fs.mkdir(testRepoPath, { recursive: true })
    await execAsync('git init', { cwd: testRepoPath })
    await execAsync('git config user.email "test@example.com"', { cwd: testRepoPath })
    await execAsync('git config user.name "Test User"', { cwd: testRepoPath })

    await fs.writeFile(path.join(testRepoPath, 'README.md'), '# Test Repo')
    await execAsync('git add .', { cwd: testRepoPath })
    await execAsync('git commit -m "Initial commit"', { cwd: testRepoPath })

    process.env['REPO_DIR'] = cloneDir
  })

  beforeEach(async () => {
    currentTaskId = `test-task-${Date.now()}`

    repositoryService = {
      getRepositoryById: vi.fn().mockResolvedValue(createRepositoryConfig()),
    } as any

    storage = {
      getTask: vi.fn().mockImplementation(async (taskId: string) => ({
        id: taskId,
        repositoryId: 'test-repo',
        workspacePath: getExpectedClonePath(taskId),
      })),
      saveTask: vi.fn(),
    } as any

    const mockEventService = {
      emitEvent: vi.fn(),
    } as any

    const mockDockerService = {} as any
    const mockClaudeService = {} as any

    previewRegistry = new PreviewServerRegistry()

    // Mock github service
    const mockGitHubService = {
      getTokenForGitOperations: vi.fn().mockResolvedValue(null),
    } as any

    workspaceService = new WorkspaceService(
      repositoryService,
      storage,
      mockEventService,
      mockDockerService,
      mockClaudeService,
      previewRegistry,
      mockGitHubService,
    )

    await fs.mkdir(cloneDir, { recursive: true })
  })

  afterEach(async () => {
    await fs.rm(cloneDir, { recursive: true, force: true })
  })

  afterAll(async () => {
    await fs.rm(testRepoPath, { recursive: true, force: true })
    await fs.rm(cloneDir, { recursive: true, force: true })
    Reflect.deleteProperty(process.env, 'REPO_DIR')
  })

  describe('clone-based workspaces', () => {
    it('creates a fresh clone for a new workspace', async () => {
      const workspace = await workspaceService.createWorkspace(currentTaskId, 'test-repo')

      const clonePath = workspace.getWorkspacePath()
      expect(clonePath).toBe(getExpectedClonePath(currentTaskId))

      const stats = await fs.stat(clonePath)
      expect(stats.isDirectory()).toBe(true)

      const gitConfig = await fs.readFile(path.join(clonePath, '.git', 'config'), 'utf-8')
      expect(gitConfig).toContain('[core]')
    })

    it('reuses an existing clone when recreating the workspace', async () => {
      const firstWorkspace = await workspaceService.createWorkspace(currentTaskId, 'test-repo')
      const clonePath = firstWorkspace.getWorkspacePath()

      await workspaceService.cleanupWorkspace(currentTaskId)

      const secondWorkspace = await workspaceService.createWorkspace(currentTaskId, 'test-repo')
      expect(secondWorkspace.getWorkspacePath()).toBe(clonePath)

      const gitHead = await fs.readFile(path.join(clonePath, '.git', 'HEAD'), 'utf-8')
      expect(gitHead.trim()).toContain(`refs/heads/workspace-${currentTaskId}`)
    })

    it('isolates clones across concurrent workspace creation', async () => {
      const taskIds = Array.from({ length: 3 }, (_, index) => `${currentTaskId}-${index}`)

      await Promise.all(
        taskIds.map(async (taskId) => {
          await workspaceService.createWorkspace(taskId, 'test-repo')
        }),
      )

      const entries = await fs.readdir(cloneDir)
      const expectedFolders = taskIds.map((taskId) => path.basename(getExpectedClonePath(taskId)))
      expect(entries.sort()).toEqual(expectedFolders.sort())
    })
  })
})
