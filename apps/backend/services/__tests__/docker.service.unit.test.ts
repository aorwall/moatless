import { beforeEach, describe, expect, it, vi } from 'vitest'
import * as portUtils from '../../utils/port-utils.js'
import { DockerService } from '../docker.service.js'
import type { SettingsService } from '../settings.service.js'

// Mock child_process at the module level
vi.mock('node:child_process', () => ({
  execSync: vi.fn(),
  exec: vi.fn((_cmd, opts, cb) => {
    const callback = typeof opts === 'function' ? opts : cb
    if (callback) {
      callback(null, { stdout: 'Docker info output', stderr: '' }, null)
    }
  }),
}))

// Mock port utilities
vi.mock('../../utils/port-utils.js')

// Mock data-dir
vi.mock('../../utils/data-dir.js', () => ({
  getDataDir: vi.fn().mockReturnValue('/test/data'),
}))

// Mock logger
vi.mock('../../utils/logger.js', () => ({
  createTaskLogger: vi.fn().mockReturnValue({
    info: vi.fn(),
    error: vi.fn(),
    warn: vi.fn(),
    debug: vi.fn(),
  }),
}))

describe('DockerService - Unit Tests', () => {
  let dockerService: DockerService
  let mockSettingsService: SettingsService

  beforeEach(() => {
    vi.clearAllMocks()

    // Mock settings service
    mockSettingsService = {
      getCombinedSettings: vi.fn().mockReturnValue({
        claudeCodeOAuthToken: 'test-token',
      }),
      getClaudeSettings: vi.fn().mockReturnValue({
        apiKey: 'test-api-key',
        model: 'claude-3',
      }),
    } as any

    // Create the service directly
    dockerService = new DockerService(mockSettingsService)
  })

  describe('Container Name Generation', () => {
    it('should generate container names correctly', () => {
      const name = dockerService.generateContainerName('test-repo', 'task-123')
      expect(name).toBe('moatless-test-repo-task-123')
    })

    it('should sanitize repository names', () => {
      const testCases = [
        { repo: 'My Repo!', task: 'task-1', expected: 'moatless-my-repo-task-1' },
        { repo: 'owner/repo', task: 'task-2', expected: 'moatless-owner-repo-task-2' },
        { repo: 'UPPER-CASE', task: 'task-3', expected: 'moatless-upper-case-task-3' },
        { repo: 'special@#$chars', task: 'task-4', expected: 'moatless-special-chars-task-4' },
      ]

      testCases.forEach(({ repo, task, expected }) => {
        expect(dockerService.generateContainerName(repo, task)).toBe(expected)
      })
    })

    it('should truncate long names', () => {
      const longRepo = 'a'.repeat(100)
      const longTask = 'b'.repeat(100)
      const name = dockerService.generateContainerName(longRepo, longTask)

      // Docker container names have a limit
      expect(name.length).toBeLessThanOrEqual(63)
      expect(name).toMatch(/^moatless-/)
    })
  })

  describe('Port Allocation', () => {
    it('should allocate ports correctly', async () => {
      vi.mocked(portUtils.getPort).mockResolvedValue(8080)

      const portMapping = await dockerService.allocateWorkerPort()

      expect(portMapping).toEqual({
        containerPort: 8001,
        hostPort: 8080,
      })
      expect(portUtils.getPort).toHaveBeenCalled()
    })

    it('should handle port allocation errors', async () => {
      vi.mocked(portUtils.getPort).mockRejectedValue(new Error('No ports available'))

      await expect(dockerService.allocateWorkerPort()).rejects.toThrow('No ports available')
    })
  })
})
