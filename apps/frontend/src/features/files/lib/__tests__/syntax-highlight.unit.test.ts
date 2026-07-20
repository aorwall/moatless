import { describe, expect, it } from 'vitest'
import { highlightLine } from '../syntax-highlight'

describe('highlightLine', () => {
  it('returns a single plain token for empty input', () => {
    expect(highlightLine('')).toEqual([{ type: 'plain', content: '' }])
  })

  it('tokenizes a keyword in a supported language', () => {
    const tokens = highlightLine('const x = 1', 'typescript')
    expect(tokens.length).toBeGreaterThan(1)
    expect(tokens.some((t) => t.type === 'keyword' && t.content === 'const')).toBe(true)
    // Round-trips to the original source text.
    expect(tokens.map((t) => t.content).join('')).toBe('const x = 1')
  })

  it('decodes HTML entities back to their characters', () => {
    const tokens = highlightLine('a < b && c > d', 'typescript')
    const joined = tokens.map((t) => t.content).join('')
    expect(joined).toContain('<')
    expect(joined).toContain('>')
    expect(joined).toContain('&')
    expect(joined).not.toContain('&lt;')
    expect(joined).not.toContain('&amp;')
  })

  it('falls back to a plain token when no language is given', () => {
    const tokens = highlightLine('plain text with no language')
    expect(tokens.map((t) => t.content).join('')).toBe('plain text with no language')
  })
})
