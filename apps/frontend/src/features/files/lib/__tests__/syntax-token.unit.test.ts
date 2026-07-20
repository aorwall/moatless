import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

// Guards against the regression where the `--hljs-*` variables consumed by
// syntax-token.tsx lost their definitions when the stylesheet that declared
// them was deleted. jsdom does not resolve CSS custom properties from
// stylesheets, so this asserts statically that every variable referenced by
// the token color map is defined for both the light (:root) and dark (.dark)
// themes in globals.css.

const componentSource = readFileSync(resolve(__dirname, '../syntax-token.tsx'), 'utf8')
const globalsCss = readFileSync(resolve(__dirname, '../../../../lib/styles/globals.css'), 'utf8')

function extractHljsVarReferences(source: string): string[] {
  const matches = source.matchAll(/var\((--hljs-[a-z-]+)\)/g)
  return [...new Set([...matches].map((m) => m[1] as string))]
}

function extractBlock(css: string, selector: string): string {
  const start = css.indexOf(`${selector} {`)
  expect(start, `selector "${selector}" not found in globals.css`).toBeGreaterThanOrEqual(0)
  const end = css.indexOf('\n}', start)
  return css.slice(start, end)
}

describe('syntax-token CSS variables', () => {
  const referenced = extractHljsVarReferences(componentSource)

  it('references at least one --hljs-* variable (sanity check)', () => {
    expect(referenced.length).toBeGreaterThan(0)
  })

  it.each([':root', '.dark'])('defines every referenced variable in %s', (selector) => {
    const block = extractBlock(globalsCss, selector)
    const missing = referenced.filter((name) => !block.includes(`${name}:`))
    expect(missing).toEqual([])
  })
})
