import { describe, expect, it } from 'vitest'
import { CreateTaskRequestSchema, CreateVibeTaskRequestSchema } from '../task.schemas.js'

describe('Task Schemas with newBranch field', () => {
  describe('CreateTaskRequestSchema', () => {
    it('should validate with all required fields only', () => {
      const validData = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
      }

      const result = CreateTaskRequestSchema.parse(validData)
      expect(result).toEqual(validData)
      expect(result.newBranch).toBeUndefined()
    })

    it('should validate with newBranch field provided', () => {
      const validData = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        newBranch: 'feature/my-custom-branch',
      }

      const result = CreateTaskRequestSchema.parse(validData)
      expect(result).toEqual(validData)
      expect(result.newBranch).toBe('feature/my-custom-branch')
    })

    it('should validate with all optional fields including newBranch', () => {
      const validData = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        message: 'Test task message',
        branchName: 'old-branch-name',
        originalBranch: 'main',
        newBranch: 'feature/new-branch',
        start: true,
        model: 'claude-3-sonnet',
        filePaths: ['/path/to/file1.js', '/path/to/file2.ts'],
      }

      const result = CreateTaskRequestSchema.parse(validData)
      expect(result).toEqual(validData)
      expect(result.newBranch).toBe('feature/new-branch')
    })

    it('should accept empty string for newBranch', () => {
      const validData = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        newBranch: '',
      }

      const result = CreateTaskRequestSchema.parse(validData)
      expect(result.newBranch).toBe('')
    })

    it('should reject invalid newBranch type', () => {
      const invalidData = {
        agentType: 'claude-code',
        repositoryId: 'test-repo-123',
        newBranch: 123, // Should be string
      }

      expect(() => CreateTaskRequestSchema.parse(invalidData)).toThrow()
    })

    it('should reject missing required fields', () => {
      const invalidData = {
        newBranch: 'feature/branch',
        // Missing agentType and repositoryId
      }

      expect(() => CreateTaskRequestSchema.parse(invalidData)).toThrow()
    })
  })

  describe('CreateVibeTaskRequestSchema', () => {
    it('should validate with all required fields only', () => {
      const validData = {
        agentType: 'claude-code',
        templateId: 'hello-world-ui',
        repositoryName: 'my-new-repo',
      }

      const result = CreateVibeTaskRequestSchema.parse(validData)
      expect(result).toEqual(validData)
      expect(result.newBranch).toBeUndefined()
    })

    it('should validate with newBranch field provided', () => {
      const validData = {
        agentType: 'claude-code',
        templateId: 'hello-world-ui',
        repositoryName: 'my-new-repo',
        newBranch: 'feature/initial-setup',
      }

      const result = CreateVibeTaskRequestSchema.parse(validData)
      expect(result).toEqual(validData)
      expect(result.newBranch).toBe('feature/initial-setup')
    })

    it('should validate with all optional fields including newBranch', () => {
      const validData = {
        agentType: 'claude-code',
        templateId: 'hello-world-ui',
        repositoryName: 'my-new-repo',
        message: 'Initialize project with custom setup',
        newBranch: 'feature/custom-init',
        start: false,
        model: 'claude-3-haiku',
        filePaths: ['/templates/config.json'],
      }

      const result = CreateVibeTaskRequestSchema.parse(validData)
      expect(result).toEqual(validData)
      expect(result.newBranch).toBe('feature/custom-init')
    })

    it('should accept various branch name formats', () => {
      const testCases = [
        'feature/new-feature',
        'bugfix/issue-123',
        'hotfix/critical-fix',
        'develop',
        'main',
        'user/feature-branch',
        'v1.0.0-release',
      ]

      for (const branchName of testCases) {
        const validData = {
          agentType: 'claude-code',
          templateId: 'hello-world-ui',
          repositoryName: 'my-new-repo',
          newBranch: branchName,
        }

        const result = CreateVibeTaskRequestSchema.parse(validData)
        expect(result.newBranch).toBe(branchName)
      }
    })

    it('should reject invalid newBranch type', () => {
      const invalidData = {
        agentType: 'claude-code',
        templateId: 'hello-world-ui',
        repositoryName: 'my-new-repo',
        newBranch: ['feature', 'branch'], // Should be string, not array
      }

      expect(() => CreateVibeTaskRequestSchema.parse(invalidData)).toThrow()
    })

    it('should reject missing required fields', () => {
      const invalidData = {
        newBranch: 'feature/branch',
        // Missing agentType, templateId, and repositoryName
      }

      expect(() => CreateVibeTaskRequestSchema.parse(invalidData)).toThrow()
    })
  })

  describe('Backward compatibility', () => {
    it('should maintain backward compatibility for CreateTaskRequestSchema without newBranch', () => {
      // This represents existing code that doesn't use newBranch
      const existingData = {
        agentType: 'claude-code',
        repositoryId: 'existing-repo',
        message: 'Existing task',
        branchName: 'old-branch',
        start: true,
      }

      const result = CreateTaskRequestSchema.parse(existingData)
      expect(result).toEqual(existingData)
      expect(result.newBranch).toBeUndefined()
    })

    it('should maintain backward compatibility for CreateVibeTaskRequestSchema without newBranch', () => {
      const existingData = {
        agentType: 'claude-code',
        templateId: 'hello-world-ui',
        repositoryName: 'existing-vibe-repo',
        message: 'Existing vibe task',
        start: true,
      }

      const result = CreateVibeTaskRequestSchema.parse(existingData)
      expect(result).toEqual(existingData)
      expect(result.newBranch).toBeUndefined()
    })
  })
})
