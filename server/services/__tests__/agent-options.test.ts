import { describe, expect, it } from 'vitest'
import {
  AgentOptionsBuilder,
  createClaudeCodeOptions,
  defaultClaudeCodeOptions,
  validateAgentOptions,
} from '../agents/agent-options.js'

describe('Agent Options', () => {
  describe('AgentOptionsBuilder', () => {
    it('should build agent options with fluent API', () => {
      const options = new AgentOptionsBuilder()
        .model('claude-3-5-sonnet-20241022')
        .apiKey('test-key')
        .maxTokens(1000)
        .temperature(0.5)
        .timeout(120)
        .systemPrompt('Test prompt')
        .tools(['bash', 'edit'])
        .outputFormat('json')
        .build()

      expect(options.model).toBe('claude-3-5-sonnet-20241022')
      expect(options.apiKey).toBe('test-key')
      expect(options.maxTokens).toBe(1000)
      expect(options.temperature).toBe(0.5)
      expect(options.timeout).toBe(120)
      expect(options.systemPrompt).toBe('Test prompt')
      expect(options.tools).toEqual(['bash', 'edit'])
      expect(options.outputFormat).toBe('json')
    })
  })

  describe('createClaudeCodeOptions', () => {
    it('should use default options', () => {
      const options = createClaudeCodeOptions()

      expect(options.model).toBe(defaultClaudeCodeOptions.model)
      expect(options.maxTokens).toBe(defaultClaudeCodeOptions.maxTokens)
      expect(options.temperature).toBe(defaultClaudeCodeOptions.temperature)
      expect(options.timeout).toBe(defaultClaudeCodeOptions.timeout)
      expect(options.tools).toEqual(defaultClaudeCodeOptions.tools)
    })

    it('should override default options', () => {
      const options = createClaudeCodeOptions({
        model: 'claude-3-haiku-20240307',
        maxTokens: 2000,
        temperature: 0.8,
      })

      expect(options.model).toBe('claude-3-haiku-20240307')
      expect(options.maxTokens).toBe(2000)
      expect(options.temperature).toBe(0.8)
      // Defaults should still be preserved
      expect(options.timeout).toBe(defaultClaudeCodeOptions.timeout)
      expect(options.tools).toEqual(defaultClaudeCodeOptions.tools)
    })
  })

  describe('validateAgentOptions', () => {
    it('should validate valid options', () => {
      const errors = validateAgentOptions({
        maxTokens: 1000,
        temperature: 0.5,
        timeout: 60,
      })

      expect(errors).toHaveLength(0)
    })

    it('should validate maxTokens range', () => {
      const errors1 = validateAgentOptions({ maxTokens: 0 })
      expect(errors1).toContain('maxTokens must be between 1 and 200000')

      const errors2 = validateAgentOptions({ maxTokens: 300000 })
      expect(errors2).toContain('maxTokens must be between 1 and 200000')
    })

    it('should validate temperature range', () => {
      const errors1 = validateAgentOptions({ temperature: -0.1 })
      expect(errors1).toContain('temperature must be between 0 and 2')

      const errors2 = validateAgentOptions({ temperature: 2.1 })
      expect(errors2).toContain('temperature must be between 0 and 2')
    })

    it('should validate timeout minimum', () => {
      const errors = validateAgentOptions({ timeout: 0 })
      expect(errors).toContain('timeout must be at least 1 second')
    })
  })
})
