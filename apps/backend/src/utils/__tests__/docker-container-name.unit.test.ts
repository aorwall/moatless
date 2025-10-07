import { describe, expect, it } from 'vitest'
import { generateContainerName } from '../docker-container-name.js'

describe('generateContainerName', () => {
  it('should generate valid container names for simple cases', () => {
    const result = generateContainerName('my-repo', 'task-12345678-abcd-efgh-ijkl-mnopqrstuvwx')
    expect(result).toBe('moatless-my-repo-task-123')
  })

  it('should sanitize repository names with special characters', () => {
    const result = generateContainerName(
      'My Repo/With Spaces!',
      'abcd1234-5678-9012-3456-789012345678',
    )
    expect(result).toBe('moatless-my-repo-with-spaces-abcd1234')
  })

  it('should handle GitHub-style repository names', () => {
    const result = generateContainerName('owner/repo-name', 'xyz789ab-cdef-0123-4567-890123456789')
    expect(result).toBe('moatless-owner-repo-name-xyz789ab')
  })

  it('should respect Docker container name length limits', () => {
    const longRepoName =
      'very-long-repository-name-that-exceeds-normal-limits-and-would-break-docker'
    const taskId = 'task1234-5678-9012-3456-789012345678'

    const result = generateContainerName(longRepoName, taskId)

    expect(result.length).toBeLessThanOrEqual(63)
    expect(result).toMatch(/^moatless-.*-task1234$/)
  })

  it('should handle repository names with underscores and dots', () => {
    const result = generateContainerName(
      'my_repo.with.dots',
      'abc12345-6789-0123-4567-890123456789',
    )
    expect(result).toBe('moatless-my_repo.with.dots-abc12345')
  })

  it('should remove leading and trailing special characters', () => {
    const result = generateContainerName('_repo-name_', 'def67890-1234-5678-9012-345678901234')
    expect(result).toBe('moatless-repo-name-def67890')
  })

  it('should collapse multiple consecutive special characters', () => {
    const result = generateContainerName(
      'repo---with___many....specials',
      'ghi09876-5432-1098-7654-321098765432',
    )
    expect(result).toBe('moatless-repo-with-many-specials-ghi09876')
  })

  it('should handle very short task IDs', () => {
    const result = generateContainerName('repo', 'abc')
    expect(result).toBe('moatless-repo-abc')
  })

  it('should handle empty repository names gracefully', () => {
    const result = generateContainerName('', 'task1234-5678-9012-3456-789012345678')
    expect(result).toBe('moatless--task1234')
  })

  it('should ensure result starts and ends with alphanumeric characters', () => {
    const testCases = [
      'normal-repo',
      '___special-repo___',
      'repo.with.dots',
      'repo_with_underscores',
    ]

    for (const repoName of testCases) {
      const result = generateContainerName(repoName, 'task1234-5678-9012-3456-789012345678')

      // Should not start or end with special characters
      expect(result).toMatch(/^[a-z0-9].*[a-z0-9]$/)
      // Should be valid Docker container name
      expect(result).toMatch(/^[a-z0-9][a-z0-9._-]*[a-z0-9]$/)
    }
  })

  it('should generate consistent names for the same inputs', () => {
    const repoName = 'consistent-repo'
    const taskId = 'same1234-5678-9012-3456-789012345678'

    const result1 = generateContainerName(repoName, taskId)
    const result2 = generateContainerName(repoName, taskId)

    expect(result1).toBe(result2)
  })

  it('should use only first 8 characters of task ID', () => {
    const result1 = generateContainerName('repo', 'abcdefgh-1234-5678-9012-345678901234')
    const result2 = generateContainerName('repo', 'abcdefgh-9999-8888-7777-666666666666')

    expect(result1).toBe('moatless-repo-abcdefgh')
    expect(result2).toBe('moatless-repo-abcdefgh')
  })

  it('should handle maximum length edge case', () => {
    // Create a repo name that will result in exactly 63 characters
    const repoName = 'repo-that-is-exactly-the-right-length-for-testing-edge'
    const taskId = 'task1234-5678-9012-3456-789012345678'

    const result = generateContainerName(repoName, taskId)

    expect(result.length).toBeLessThanOrEqual(63)
    expect(result).toMatch(/^moatless-.*-task1234$/)
  })
})
