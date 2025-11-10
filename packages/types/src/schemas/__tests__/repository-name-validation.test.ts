import { describe, expect, it } from 'vitest'
import { CreateLocalRepositorySchema } from '../repository.schemas'

describe('repository name validation', () => {
  it('should accept names with spaces', () => {
    const result = CreateLocalRepositorySchema.safeParse({
      kind: 'local',
      name: 'Hello World UI',
      path: '/path/to/repo',
    })
    expect(result.success).toBe(true)
  })

  it('should accept names with special characters', () => {
    const testNames = [
      'My Project 2024!',
      'Test & Dev Env',
      'Repository (v2)',
      'special@chars#here',
    ]

    for (const name of testNames) {
      const result = CreateLocalRepositorySchema.safeParse({
        kind: 'local',
        name,
        path: '/path/to/repo',
      })
      expect(result.success).toBe(true)
    }
  })

  it('should accept traditional naming patterns', () => {
    const testNames = ['hello-world-ui', 'test_repo', 'owner/repo', '123-numeric-start']

    for (const name of testNames) {
      const result = CreateLocalRepositorySchema.safeParse({
        kind: 'local',
        name,
        path: '/path/to/repo',
      })
      expect(result.success).toBe(true)
    }
  })

  it('should reject empty names', () => {
    const result = CreateLocalRepositorySchema.safeParse({
      kind: 'local',
      name: '',
      path: '/path/to/repo',
    })
    expect(result.success).toBe(false)
  })

  it('should reject names longer than 255 characters', () => {
    const result = CreateLocalRepositorySchema.safeParse({
      kind: 'local',
      name: 'a'.repeat(256),
      path: '/path/to/repo',
    })
    expect(result.success).toBe(false)
  })

  it('should accept names with exactly 255 characters', () => {
    const result = CreateLocalRepositorySchema.safeParse({
      kind: 'local',
      name: 'a'.repeat(255),
      path: '/path/to/repo',
    })
    expect(result.success).toBe(true)
  })
})
