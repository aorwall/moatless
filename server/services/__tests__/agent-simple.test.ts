import type { Event } from '@shared/types/event.js'
import { describe, expect, it, vi } from 'vitest'
import { AgentOptionsBuilder, validateAgentOptions } from '../agents/agent-options.js'
import { ClaudeCodeAgent } from '../agents/claude-code-agent.js'

// Mock readline module
vi.mock('readline', () => ({
  createInterface: vi.fn(() => ({
    [Symbol.asyncIterator]: async function* () {
      yield JSON.stringify({ type: 'start', message: 'Starting' })
      yield JSON.stringify({ type: 'text', content: 'Hello world' })
      yield JSON.stringify({ type: 'completion', exitCode: 0 })
    },
    close: vi.fn(),
  })),
}))

describe('Claude Code Agent', () => {
  describe('buildCliArgs', () => {
    it('should build CLI arguments correctly', () => {
      const agent = new ClaudeCodeAgent('test-task', {
        model: 'claude-3-haiku-20240307',
        maxTokens: 1000,
        temperature: 0.5,
        systemPrompt: 'Test prompt',
        tools: ['bash', 'edit'],
        outputFormat: 'json',
      })

      // Access private method through reflection
      const args = (agent as any).buildCliArgs('Hello world')

      expect(args).toContain('--model')
      expect(args).toContain('claude-3-haiku-20240307')
      expect(args).toContain('--max-tokens')
      expect(args).toContain('1000')
      expect(args).toContain('--temperature')
      expect(args).toContain('0.5')
      expect(args).toContain('--system')
      expect(args).toContain('Test prompt')
      expect(args).toContain('--tool')
      expect(args).toContain('bash')
      expect(args).toContain('--tool')
      expect(args).toContain('edit')
      expect(args).toContain('--output')
      expect(args).toContain('json')
      expect(args).toContain('Hello world')
    })
  })

  describe('event parsing', () => {
    it('should parse Claude CLI events correctly', async () => {
      const agent = new ClaudeCodeAgent('test-task', {
        model: 'claude-3-haiku-20240307',
      })

      // Mock spawn
      const mockProcess = {
        stdout: {
          on: vi.fn(),
          pipe: vi.fn(),
        },
        stderr: {
          on: vi.fn(),
        },
        on: vi.fn((event, handler) => {
          if (event === 'close') {
            setTimeout(() => handler(0), 10)
          }
        }),
        kill: vi.fn(),
        killed: false,
      }

      vi.spyOn(require('child_process'), 'spawn').mockReturnValue(mockProcess as any)

      const events: Event[] = []
      for await (const event of agent.run('Hello world')) {
        events.push(event)
      }

      expect(events).toHaveLength(4) // start + 3 from mock
      expect(events[0].type).toBe('start')
      expect(events[1].type).toBe('start') // Mapped from mock
      expect(events[2].type).toBe('message') // Mapped from 'text'
      expect(events[3].type).toBe('completion')
    })
  })
})

describe('Agent Options Builder', () => {
  it('should build options with fluent API', () => {
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

describe('Agent Options Validation', () => {
  it('should validate options correctly', () => {
    expect(validateAgentOptions({ maxTokens: 1000 })).toHaveLength(0)
    expect(validateAgentOptions({ maxTokens: 0 })).toContain(
      'maxTokens must be between 1 and 200000',
    )
    expect(validateAgentOptions({ temperature: -0.1 })).toContain(
      'temperature must be between 0 and 2',
    )
    expect(validateAgentOptions({ timeout: 0 })).toContain('timeout must be at least 1 second')
  })
})
