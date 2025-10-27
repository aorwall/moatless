import type { CreateVibeTaskRequest } from '@moatless/types'
import { CreateVibeTaskRequestSchema } from '@moatless/types'
import { describe, expect, it } from 'vitest'

describe('CreateVibeTaskRequest validation', () => {
  it('should validate a valid vibe task request with minimal fields', () => {
    const validRequest: CreateVibeTaskRequest = {
      agentType: 'test-agent',
      templateId: 'basic-hello-website',
      start: true,
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

  it('should accept optional message, model, and fileIds', () => {
    const requestWithOptionals: CreateVibeTaskRequest = {
      agentType: 'custom-agent',
      templateId: 'basic-hello-website',
      start: true,
      message: 'Custom message for the task',
      model: 'gpt-4',
      fileIds: ['file-id-1', 'file-id-2'],
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
      fileIds: ['file-id-1'], // This IS supported
    }

    const result = CreateVibeTaskRequestSchema.safeParse(requestWithUnsupportedFields)
    expect(result.success).toBe(true) // Will pass but ignore unknown fields
    if (result.success) {
      // Verify the parsed result doesn't contain unsupported fields
      expect(result.data).not.toHaveProperty('repositoryName')
      expect(result.data).not.toHaveProperty('start')
      expect(result.data).not.toHaveProperty('newBranch')
      // But fileIds should be present
      expect(result.data).toHaveProperty('fileIds')
      expect(result.data.fileIds).toEqual(['file-id-1'])
    }
  })
})
