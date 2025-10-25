import { describe, expect, it } from 'vitest'
import { calculateContextTokens } from '../context-tokens'

describe('calculateContextTokens', () => {
  it('should return undefined for messages without usage', () => {
    const payload = {
      type: 'system',
      text: 'Hello world',
    } as any

    expect(calculateContextTokens(payload)).toBeUndefined()
  })

  it('should return undefined for messages without message property', () => {
    const payload = {
      type: 'result',
      content: 'Some result',
    } as any

    expect(calculateContextTokens(payload)).toBeUndefined()
  })

  it('should calculate context tokens correctly', () => {
    const payload = {
      type: 'assistant',
      message: {
        text: 'Hello',
        usage: {
          input_tokens: 100,
          cache_read_input_tokens: 50,
          cache_creation_input_tokens: 25,
          output_tokens: 75,
        },
      },
    } as any

    const result = calculateContextTokens(payload)
    expect(result).toBe(175) // 100 + 50 + 25
  })

  it('should handle missing usage properties', () => {
    const payload = {
      type: 'assistant',
      message: {
        text: 'Hello',
        usage: {
          input_tokens: 100,
          // Missing cache tokens
          output_tokens: 75,
        },
      },
    } as any

    const result = calculateContextTokens(payload)
    expect(result).toBe(100) // Only input_tokens
  })

  it('should handle null usage', () => {
    const payload = {
      type: 'assistant',
      message: {
        text: 'Hello',
        usage: null,
      },
    } as any

    const result = calculateContextTokens(payload)
    expect(result).toBeUndefined()
  })
})
