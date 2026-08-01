import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import type { HtmlTagDescriptor, Plugin } from 'vite'

export interface InspectorPluginOptions {
  /** Static server ID for message identification (default: 'vite-dev') */
  serverId?: string
  /** Only inject in development mode (default: true) */
  devOnly?: boolean
  /**
   * Also inject the T3 Code annotation client, which lets T3 Code pick, draw on
   * and comment about elements in this page (default: true).
   *
   * It is a second ~80 kB of inlined script and it only wakes up for a host
   * that speaks its protocol, so a project previewed solely in the Moatless UI
   * can turn it off and lose nothing.
   */
  annotation?: boolean
}

/** Read one of the prebuilt clients that sit next to this file in `dist/`. */
function readClient(fileName: string): string {
  const here = path.dirname(fileURLToPath(import.meta.url))
  return readFileSync(path.join(here, fileName), 'utf-8')
}

export default function inspectorPlugin(options: InspectorPluginOptions = {}): Plugin {
  const { devOnly = true, serverId = 'vite-dev', annotation = true } = options
  let isDev = true

  return {
    name: 'moatless-inspector',

    configResolved(config) {
      isDev = config.mode === 'development'
    },

    transformIndexHtml(): HtmlTagDescriptor[] | undefined {
      if (devOnly && !isDev) {
        return
      }

      const tags: HtmlTagDescriptor[] = [
        {
          tag: 'script',
          children: readClient('client.js').replace(/__SERVER_ID__/g, serverId),
          injectTo: 'body',
        },
      ]

      // Both clients talk to whatever framed the page, over postMessage, and
      // each ignores what it does not recognise — so a page carrying both is
      // annotatable from T3 Code and inspectable from the Moatless UI without
      // either knowing about the other.
      if (annotation) {
        tags.push({
          tag: 'script',
          children: readClient('t3-annotation-client.js'),
          injectTo: 'body',
        })
      }

      return tags
    },
  }
}
