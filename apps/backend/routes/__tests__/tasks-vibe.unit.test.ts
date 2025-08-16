import type { CreateVibeTaskRequest } from '@moatless/types'
import { CreateVibeTaskRequestSchema } from '@moatless/types'
import { describe, expect, it } from 'vitest'

describe('CreateVibeTaskRequest validation', () => {
  it('should validate a valid vibe task request with minimal fields', () => {
    const validRequest: CreateVibeTaskRequest = {
      agentType: 'test-agent',
      templateId: 'basic-hello-website',
    }

    const result = CreateVibeTaskRequestSchema.safeParse(validRequest)
    expect(result.success).toBe(true)
  })

  it('should reject request missing required fields', () => {
    const invalidRequest = {
      agentType: 'test-agent',
      // Missing templateId
    }

    const result = CreateVibeTaskRequestSchema.safeParse(invalidRequest)
    expect(result.success).toBe(false)
  })

  it('should accept optional message, model, and filePaths', () => {
    const requestWithOptionals: CreateVibeTaskRequest = {
      agentType: 'custom-agent',
      templateId: 'basic-hello-website',
      message: 'Custom message for the task',
      model: 'gpt-4',
      filePaths: ['/path/to/file1.txt', '/path/to/file2.txt'],
    }

    const result = CreateVibeTaskRequestSchema.safeParse(requestWithOptionals)
    expect(result.success).toBe(true)
  })

  it('should reject fields that are no longer supported', () => {
    const requestWithUnsupportedFields = {
      agentType: 'test-agent',
      templateId: 'basic-hello-website',
      repositoryName: 'should-not-be-here', // No longer in schema
      start: true, // No longer in schema
      newBranch: 'feature/test', // No longer in schema
      filePaths: ['/path/to/file.txt'], // This IS supported
    }

    const result = CreateVibeTaskRequestSchema.safeParse(requestWithUnsupportedFields)
    expect(result.success).toBe(true) // Will pass but ignore unknown fields
    if (result.success) {
      // Verify the parsed result doesn't contain unsupported fields
      expect(result.data).not.toHaveProperty('repositoryName')
      expect(result.data).not.toHaveProperty('start')
      expect(result.data).not.toHaveProperty('newBranch')
      // But filePaths should be present
      expect(result.data).toHaveProperty('filePaths')
      expect(result.data.filePaths).toEqual(['/path/to/file.txt'])
    }
  })
})
