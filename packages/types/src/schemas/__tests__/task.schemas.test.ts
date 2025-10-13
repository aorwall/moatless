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
        fileIds: ['file-id-1', 'file-id-2'],
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
      }

      const result = CreateVibeTaskRequestSchema.parse(validData)
      expect(result).toEqual({
        ...validData,
        start: true, // Default value
      })
      expect(result.message).toBeUndefined()
      expect(result.model).toBeUndefined()
      expect(result.fileIds).toBeUndefined()
    })

    it('should validate with optional message and model', () => {
      const validData = {
        agentType: 'claude-code',
        templateId: 'hello-world-ui',
        message: 'Build a dashboard',
        model: 'claude-3-haiku',
      }

      const result = CreateVibeTaskRequestSchema.parse(validData)
      expect(result).toEqual({
        ...validData,
        start: true, // Default value
      })
      expect(result.message).toBe('Build a dashboard')
      expect(result.model).toBe('claude-3-haiku')
    })

    it('should validate with all optional fields', () => {
      const validData = {
        agentType: 'claude-code',
        templateId: 'hello-world-ui',
        message: 'Initialize project with custom setup',
        model: 'claude-3-haiku',
        fileIds: ['file-id-config-json'],
      }

      const result = CreateVibeTaskRequestSchema.parse(validData)
      expect(result).toEqual({
        ...validData,
        start: true, // Default value
      })
      expect(result.fileIds).toEqual(['file-id-config-json'])
    })

    it('should accept various file IDs', () => {
      const validData = {
        agentType: 'claude-code',
        templateId: 'hello-world-ui',
        fileIds: ['file-id-1', 'file-id-2', 'file-id-3'],
      }

      const result = CreateVibeTaskRequestSchema.parse(validData)
      expect(result.fileIds).toEqual(['file-id-1', 'file-id-2', 'file-id-3'])
    })

    it('should reject invalid fileIds type', () => {
      const invalidData = {
        agentType: 'claude-code',
        templateId: 'hello-world-ui',
        fileIds: 'not-an-array', // Should be array
      }

      expect(() => CreateVibeTaskRequestSchema.parse(invalidData)).toThrow()
    })

    it('should reject missing required fields', () => {
      const invalidData = {
        message: 'Some message',
        // Missing agentType and templateId
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

    it('should maintain backward compatibility for CreateVibeTaskRequestSchema', () => {
      const existingData = {
        agentType: 'claude-code',
        templateId: 'hello-world-ui',
        message: 'Existing vibe task',
      }

      const result = CreateVibeTaskRequestSchema.parse(existingData)
      expect(result).toEqual({
        ...existingData,
        start: true, // Default value
      })
      expect(result.fileIds).toBeUndefined()
    })
  })
})
