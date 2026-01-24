import { readFileSync } from 'node:fs'
import { afterEach, describe, expect, it, type Mock, vi } from 'vitest'
import { injectScriptIntoHtml } from '../server/html-injector'

vi.mock('node:fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:fs')>()
  return {
    ...actual,
    readFileSync: vi.fn(actual.readFileSync),
  }
})

describe('injectScriptIntoHtml', () => {
  it('injects script before </body>', () => {
    const html = '<html><body><div></div></body></html>'
    const result = injectScriptIntoHtml(html, 'alert(1)')
    expect(result).toBe('<html><body><div></div><script>alert(1)</script></body></html>')
  })

  it('injects script before </html> when no </body>', () => {
    const html = '<html><div></div></html>'
    const result = injectScriptIntoHtml(html, 'alert(1)')
    expect(result).toBe('<html><div></div><script>alert(1)</script></html>')
  })

  it('appends script to end when no closing tags', () => {
    const html = '<div>content</div>'
    const result = injectScriptIntoHtml(html, 'alert(1)')
    expect(result).toBe('<div>content</div><script>alert(1)</script>')
  })

  it('handles empty script content', () => {
    const html = '<html><body></body></html>'
    const result = injectScriptIntoHtml(html, '')
    expect(result).toBe('<html><body><script></script></body></html>')
  })

  it('preserves existing content in body', () => {
    const html = '<html><body><h1>Hello</h1><p>World</p></body></html>'
    const result = injectScriptIntoHtml(html, 'console.log("test")')
    expect(result).toContain('<h1>Hello</h1><p>World</p>')
    expect(result).toContain('<script>console.log("test")</script></body>')
  })
})

describe('getInspectorScript', () => {
  const MOCK_CLIENT_SCRIPT =
    'var serverId="__SERVER_ID__";postMessage({type:"INSPECTOR_SCRIPT_LOADED"})'

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('returns script with serverId replaced', async () => {
    ;(readFileSync as Mock).mockReturnValueOnce(MOCK_CLIENT_SCRIPT)

    const { getInspectorScript } = await import('../server/script-loader')
    const script = getInspectorScript({ serverId: 'test-123' })

    expect(script).toContain('test-123')
    expect(script).not.toContain('__SERVER_ID__')
    expect(script).toContain('INSPECTOR_SCRIPT_LOADED')
  })

  it('preserves __SERVER_ID__ when no serverId provided', async () => {
    ;(readFileSync as Mock).mockReturnValueOnce(MOCK_CLIENT_SCRIPT)

    const { getInspectorScript } = await import('../server/script-loader')
    const script = getInspectorScript({})

    expect(script).toContain('__SERVER_ID__')
  })

  it('replaces all occurrences of __SERVER_ID__', async () => {
    const multiIdScript = 'id1="__SERVER_ID__";id2="__SERVER_ID__"'
    ;(readFileSync as Mock).mockReturnValueOnce(multiIdScript)

    const { getInspectorScript } = await import('../server/script-loader')
    const script = getInspectorScript({ serverId: 'multi-test' })

    expect(script).toBe('id1="multi-test";id2="multi-test"')
  })
})
