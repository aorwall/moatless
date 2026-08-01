import { readFileSync } from 'node:fs'
import type { HtmlTagDescriptor, Plugin } from 'vite'
import { afterEach, beforeEach, describe, expect, it, type Mock, vi } from 'vitest'

vi.mock('node:fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:fs')>()
  return {
    ...actual,
    readFileSync: vi.fn(actual.readFileSync),
  }
})

const INSPECTOR_CLIENT =
  'var serverId="__SERVER_ID__";postMessage({type:"INSPECTOR_SCRIPT_LOADED"})'
const ANNOTATION_CLIENT =
  'var MoatlessPreviewAnnotation=(function(){/* __SERVER_ID__ is not ours */})()'

/**
 * The clients only exist next to the built plugin in `dist/`, so stand in for
 * them by which file the plugin asked for.
 */
function stubClients(): void {
  ;(readFileSync as Mock).mockImplementation((file: unknown) =>
    String(file).endsWith('annotation-client.js') ? ANNOTATION_CLIENT : INSPECTOR_CLIENT,
  )
}

/** The plugin declares its hooks as plain methods; Vite's type allows objects. */
function inject(plugin: Plugin, mode = 'development'): HtmlTagDescriptor[] | undefined {
  const configResolved = plugin.configResolved as (config: { mode: string }) => void
  configResolved.call(plugin, { mode })
  const transform = plugin.transformIndexHtml as () => HtmlTagDescriptor[] | undefined
  return transform.call(plugin)
}

describe('inspectorPlugin', () => {
  beforeEach(() => {
    stubClients()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('injects the inspector and annotation clients into the body', async () => {
    const { default: inspectorPlugin } = await import('../vite')

    const tags = inject(inspectorPlugin())

    expect(tags).toHaveLength(2)
    expect(tags?.[0]).toMatchObject({ tag: 'script', injectTo: 'body' })
    expect(tags?.[0]?.children).toContain('INSPECTOR_SCRIPT_LOADED')
    expect(tags?.[1]).toMatchObject({ tag: 'script', injectTo: 'body' })
    expect(tags?.[1]?.children).toContain('MoatlessPreviewAnnotation')
  })

  it('substitutes the serverId in the inspector client only', async () => {
    const { default: inspectorPlugin } = await import('../vite')

    const tags = inject(inspectorPlugin({ serverId: 'sandbox-42' }))

    expect(tags?.[0]?.children).toContain('sandbox-42')
    expect(tags?.[0]?.children).not.toContain('__SERVER_ID__')
    // The annotation client identifies its host by origin, not by server ID —
    // rewriting a string it happens to contain would only corrupt it.
    expect(tags?.[1]?.children).toBe(ANNOTATION_CLIENT)
  })

  it('leaves the annotation client out when asked to', async () => {
    const { default: inspectorPlugin } = await import('../vite')

    const tags = inject(inspectorPlugin({ annotation: false }))

    expect(tags).toHaveLength(1)
    expect(tags?.[0]?.children).toContain('INSPECTOR_SCRIPT_LOADED')
  })

  it('injects nothing in a production build by default', async () => {
    const { default: inspectorPlugin } = await import('../vite')

    expect(inject(inspectorPlugin(), 'production')).toBeUndefined()
  })

  it('injects both clients in a production build when devOnly is off', async () => {
    const { default: inspectorPlugin } = await import('../vite')

    const tags = inject(inspectorPlugin({ devOnly: false }), 'production')

    expect(tags).toHaveLength(2)
  })
})
