import { beforeEach, describe, expect, it, vi } from 'vitest'

// Set up mocks before any imports
vi.mock('node:util', () => {
  const mockExecAsync = vi.fn()
  return {
    promisify: () => mockExecAsync,
    __mockExecAsync: mockExecAsync, // Export for test access
  }
})

vi.mock('node:child_process', () => ({
  exec: vi.fn(),
}))

// Import after mocks are set up
import {
  containerExists,
  generateContainerName,
  getContainerIdByName,
  getContainerStatus,
  validateContainerLabels,
} from '../docker-container.js'

describe('Docker Container Utilities', () => {
  let mockExecAsync: any

  beforeEach(async () => {
    // Get the mock from the mocked module
    const util = vi.mocked(await import('node:util'))
    mockExecAsync = (util as any).__mockExecAsync
    vi.clearAllMocks()
  })

  describe('generateContainerName', () => {
    it('should generate container name with repository name and task ID suffix', () => {
      const result = generateContainerName('my-repo', 'task-12345678901234567890')
      expect(result).toBe('moatless-my-repo-task-123')
    })

    it('should handle GitHub-style repository names', () => {
      const result = generateContainerName('aorwall/moatless', 'task-12345678901234567890')
      expect(result).toBe('moatless-aorwall-moatless-task-123')
    })

    it('should sanitize repository names with special characters', () => {
      const result = generateContainerName('My_Repo@#$%^&*()', 'task-12345678901234567890')
      expect(result).toBe('moatless-my-repo-task-123')
    })

    it('should handle very long repository names by truncating', () => {
      const longName =
        'this-is-a-very-long-repository-name-that-exceeds-normal-length-limits-and-should-be-truncated-properly-to-fit-within-docker-container-naming-constraints-which-have-a-maximum-of-255-characters-total-for-the-container-name'
      const result = generateContainerName(longName, 'task-12345678901234567890')

      expect(result.length).toBeLessThanOrEqual(255)
      expect(result).toMatch(/^moatless-.+-task-123$/)
      expect(result).not.toMatch(/-$/) // Should not end with hyphen after truncation
    })

    it('should handle empty repository names', () => {
      const result = generateContainerName('', 'task-12345678901234567890')
      expect(result).toBe('moatless--task-123')
    })

    it('should handle repository names with only special characters', () => {
      const result = generateContainerName('###---___...', 'task-12345678901234567890')
      expect(result).toBe('moatless--task-123')
    })

    it('should handle short task IDs', () => {
      const result = generateContainerName('my-repo', 'abc')
      expect(result).toBe('moatless-my-repo-abc')
    })

    it('should use exactly 8 characters from task ID', () => {
      const result = generateContainerName('my-repo', 'task-12345678901234567890')
      expect(result).toBe('moatless-my-repo-task-123')
    })
  })

  describe('containerExists', () => {
    it('should return true when container exists', async () => {
      mockExecAsync.mockResolvedValue({ stdout: 'container-id', stderr: '' })

      const result = await containerExists('test-container')
      expect(result).toBe(true)
    })

    it('should return false when container does not exist', async () => {
      mockExecAsync.mockRejectedValue(new Error('No such container'))

      const result = await containerExists('nonexistent-container')
      expect(result).toBe(false)
    })
  })

  describe('getContainerStatus', () => {
    it('should return "running" for running containers', async () => {
      mockExecAsync.mockResolvedValue({ stdout: 'running\n', stderr: '' })

      const result = await getContainerStatus('running-container')
      expect(result).toBe('running')
    })

    it('should return "stopped" for exited containers', async () => {
      mockExecAsync.mockResolvedValue({ stdout: 'exited\n', stderr: '' })

      const result = await getContainerStatus('stopped-container')
      expect(result).toBe('stopped')
    })

    it('should return "not-found" when container does not exist', async () => {
      mockExecAsync.mockRejectedValue(new Error('No such container'))

      const result = await getContainerStatus('nonexistent-container')
      expect(result).toBe('not-found')
    })
  })

  describe('getContainerIdByName', () => {
    it('should return container ID when container exists', async () => {
      mockExecAsync.mockResolvedValue({ stdout: 'abc123def456\n', stderr: '' })

      const result = await getContainerIdByName('test-container')
      expect(result).toBe('abc123def456')
    })

    it('should return null when container does not exist', async () => {
      mockExecAsync.mockRejectedValue(new Error('No such container'))

      const result = await getContainerIdByName('nonexistent-container')
      expect(result).toBe(null)
    })
  })

  describe('validateContainerLabels', () => {
    it('should return true when container has correct task ID label', async () => {
      mockExecAsync.mockResolvedValue({ stdout: 'test-task-123\n', stderr: '' })

      const result = await validateContainerLabels('test-container', 'test-task-123')
      expect(result).toBe(true)
    })

    it('should return false when container has incorrect task ID label', async () => {
      mockExecAsync.mockResolvedValue({ stdout: 'different-task-456\n', stderr: '' })

      const result = await validateContainerLabels('test-container', 'test-task-123')
      expect(result).toBe(false)
    })

    it('should return false when container does not exist', async () => {
      mockExecAsync.mockRejectedValue(new Error('No such container'))

      const result = await validateContainerLabels('nonexistent-container', 'test-task-123')
      expect(result).toBe(false)
    })
  })
})
