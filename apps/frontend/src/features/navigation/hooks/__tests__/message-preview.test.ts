import { describe, expect, it } from 'vitest'
import { formatPreview } from '../message-preview'

describe('formatPreview', () => {
  it('collapses whitespace', () => {
    expect(formatPreview('a\n\n  b   c')).toBe('a b c')
  })

  it('truncates with an ellipsis', () => {
    const preview = formatPreview('x'.repeat(200))
    expect(preview).toHaveLength(141)
    expect(preview.endsWith('…')).toBe(true)
  })
})
