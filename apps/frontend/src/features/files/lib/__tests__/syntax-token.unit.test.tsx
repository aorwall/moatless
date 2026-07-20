import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { SyntaxToken } from '../syntax-token'

describe('SyntaxToken', () => {
  it('applies the themed CSS variable color for a known token type', () => {
    const { container } = render(<SyntaxToken token={{ type: 'keyword', content: 'const' }} />)
    const span = container.querySelector('span')
    expect(span?.textContent).toBe('const')
    // The color must reference the live --hljs-* variable (defined in globals.css),
    // not a hard-coded value — this is what makes highlighting render at all.
    expect(span?.style.color).toBe('var(--hljs-keyword)')
  })

  it('renders plain tokens without a color style', () => {
    const { container } = render(<SyntaxToken token={{ type: 'plain', content: ' x = ' }} />)
    const span = container.querySelector('span')
    expect(span?.textContent).toBe(' x = ')
    expect(span?.style.color).toBe('')
  })

  it('renders unknown token types without a color style', () => {
    const { container } = render(
      <SyntaxToken token={{ type: 'not-a-real-type', content: 'foo' }} />,
    )
    const span = container.querySelector('span')
    expect(span?.textContent).toBe('foo')
    expect(span?.style.color).toBe('')
  })
})
