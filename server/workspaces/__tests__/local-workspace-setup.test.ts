// import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
// import { promisify } from 'node:util'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Environment } from '../../../shared/types/environment.js'
import type { EventService } from '../../services/event.service.js'
import { LocalWorkspace } from '../local.js'

// const _execAsync = promisify(exec)

// Mock dependencies
vi.mock('../../utils/logger.js', () => ({
  createTaskLogger: vi.fn(() => ({
    info: vi.fn(),
    error: vi.fn(),
    warn: vi.fn(),
    debug: vi.fn(),
  })),
}))

vi.mock('../../utils/git-utils.js', () => ({
  getCurrentBranch: vi.fn(),
  getGitDiffString: vi.fn().mockResolvedValue(['', []]),
  hasUncommittedChanges: vi.fn().mockResolvedValue(false),
  isGitRepository: vi.fn().mockResolvedValue(true),
}))

// Mock child_process
vi.mock('node:child_process', () => ({
  exec: vi.fn((_cmd, _opts, cb) => {
    if (cb) cb(null, { stdout: '', stderr: '' })
  }),
}))

vi.mock('node:util', () => ({
  promisify: vi.fn((_fn) => {
    return vi.fn(async (command: string, _options?: unknown) => {
      if (command.includes('git show-ref')) {
        throw new Error('Branch not found')
      }
      if (command.includes('git worktree add')) {
        return { stdout: '', stderr: '' }
      }
      if (command === 'echo "test setup"') {
        return { stdout: 'test setup', stderr: '' }
      }
      return { stdout: '', stderr: '' }
    })
  }),
}))

describe('LocalWorkspace - Setup with Missing Path', () => {
  let mockEventService: EventService
  const taskId = 'test-task-123'
  const testWorkspacePath = '/tmp/test-repo-workspace-test-task-123'
  const mockEnvironment = {
    id: 'env-123',
    name: 'Test Environment',
    description: 'Test environment',
    repository: {
      type: 'local' as const,
      path: '/tmp/test-repo',
      useGitWorktrees: true,
    },
    codeExecution: {
      type: 'local' as const,
      setupCommands: 'echo "test setup"',
    },
  }

  beforeEach(async () => {
    // Setup mock event service
    mockEventService = {
      emitEvent: vi.fn(),
    } as EventService

    // Clean up test directories if they exist
    try {
      await fs.rm(testWorkspacePath, { recursive: true, force: true })
    } catch {}
  })

  afterEach(async () => {
    vi.clearAllMocks()
    // Clean up test directories
    try {
      await fs.rm(testWorkspacePath, { recursive: true, force: true })
    } catch {}
  })

  it('should recreate workspace when path does not exist', async () => {
    // Create workspace with non-existent path
    const workspace = new LocalWorkspace(
      taskId,
      mockEnvironment as Environment,
      mockEventService,
      undefined,
      undefined,
      testWorkspacePath, // This path doesn't exist
    )

    // Spy on the recreateGitWorktree method
    const recreateSpy = vi
      .spyOn(
        workspace as LocalWorkspace & { recreateGitWorktree: () => Promise<void> },
        'recreateGitWorktree',
      )
      .mockImplementation(async () => {
        // Simulate creating the directory
        await fs.mkdir(testWorkspacePath, { recursive: true })
      })

    // Mock runCommand for setup commands
    vi.spyOn(workspace, 'runCommand').mockResolvedValue({
      command: 'echo "test setup"',
      output: 'test setup',
      exitCode: 0,
      durationMs: 100,
    })

    // Setup should recreate the workspace
    await workspace.setup()

    // Verify recreateGitWorktree was called
    expect(recreateSpy).toHaveBeenCalled()

    // Verify the workspace was created
    const exists = await fs
      .access(testWorkspacePath)
      .then(() => true)
      .catch(() => false)
    expect(exists).toBe(true)
  })

  it('should handle setup command failures with proper error', async () => {
    // Create test directory
    await fs.mkdir(testWorkspacePath, { recursive: true })

    const workspace = new LocalWorkspace(
      taskId,
      mockEnvironment as Environment,
      mockEventService,
      undefined,
      undefined,
      testWorkspacePath,
    )

    // Mock runCommand to simulate command failure
    const mockRunCommand = vi.spyOn(workspace, 'runCommand').mockResolvedValue({
      command: 'echo "test setup"',
      output: 'Command failed',
      exitCode: 1,
      durationMs: 100,
    })

    // Setup should complete but log the failure
    await workspace.setup()

    // Verify command was attempted
    expect(mockRunCommand).toHaveBeenCalledWith('echo "test setup"')
  })

  it('should validate workspace path exists before running commands', async () => {
    const workspace = new LocalWorkspace(
      taskId,
      mockEnvironment as Environment,
      mockEventService,
      undefined,
      undefined,
      testWorkspacePath,
    )

    // Check that workspace path is not valid
    const isValid = await workspace.isWorkspacePathValid()
    expect(isValid).toBe(false)

    // Create the directory
    await fs.mkdir(testWorkspacePath, { recursive: true })

    // Now it should be valid
    const isValidAfter = await workspace.isWorkspacePathValid()
    expect(isValidAfter).toBe(true)
  })

  it('should detect when git worktree is invalid', async () => {
    // Create a directory that's not a git repo
    await fs.mkdir(testWorkspacePath, { recursive: true })

    const workspace = new LocalWorkspace(
      taskId,
      mockEnvironment as Environment,
      mockEventService,
      undefined,
      undefined,
      testWorkspacePath,
    )

    // Mock isGitRepository to return false
    const { isGitRepository } = await import('../../utils/git-utils.js')
    vi.mocked(isGitRepository).mockResolvedValueOnce(false)

    // Should return false because it's not a valid git repo
    const isValid = await workspace.isWorkspacePathValid()
    expect(isValid).toBe(false)
  })
})
