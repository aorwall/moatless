import type { CreateVibeTaskRequest } from '@moatless/types'
import { CreateVibeTaskRequestSchema } from '@moatless/types'
import { describe, expect, it } from 'vitest'

describe('CreateVibeTaskRequest validation', () => {
  it('should validate a valid vibe task request', () => {
    const validRequest: CreateVibeTaskRequest = {
      agentType: 'test-agent',
      templateId: 'basic-hello-website',
      repositoryName: 'my-test-website',
      message: 'Create a new website from template',
      start: false,
    }

    const result = CreateVibeTaskRequestSchema.safeParse(validRequest)
    expect(result.success).toBe(true)
  })

  it('should reject request missing required fields', () => {
    const invalidRequest = {
      agentType: 'test-agent',
      // Missing templateId and repositoryName
      start: false,
    }

    const result = CreateVibeTaskRequestSchema.safeParse(invalidRequest)
    expect(result.success).toBe(false)
  })

  it('should accept all optional parameters', () => {
    const requestWithOptionals: CreateVibeTaskRequest = {
      agentType: 'custom-agent',
      templateId: 'basic-hello-website',
      repositoryName: 'my-custom-website',
      message: 'Custom message for the task',
      start: true,
      model: 'gpt-4',
      filePaths: ['/path/to/file1.txt'],
    }

    const result = CreateVibeTaskRequestSchema.safeParse(requestWithOptionals)
    expect(result.success).toBe(true)
  })

  it('should validate repository name formats', () => {
    const requestWithSpecialChars: CreateVibeTaskRequest = {
      agentType: 'test-agent',
      templateId: 'basic-hello-website',
      repositoryName: 'my-website-v2.1-final',
      start: false,
    }

    const result = CreateVibeTaskRequestSchema.safeParse(requestWithSpecialChars)
    expect(result.success).toBe(true)
  })
})
