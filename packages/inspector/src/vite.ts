import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import type { HtmlTagDescriptor, Plugin } from 'vite'

export interface InspectorPluginOptions {
  /** Static server ID for message identification (default: 'vite-dev') */
  serverId?: string
  /** Only inject in development mode (default: true) */
  devOnly?: boolean
}

export default function inspectorPlugin(options: InspectorPluginOptions = {}): Plugin {
  const { devOnly = true, serverId = 'vite-dev' } = options
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

      const __filename = fileURLToPath(import.meta.url)
      const __dirname = path.dirname(__filename)
      const scriptPath = path.join(__dirname, 'client.js')
      let scriptContent = readFileSync(scriptPath, 'utf-8')

      scriptContent = scriptContent.replace(/__SERVER_ID__/g, serverId)

      return [
        {
          tag: 'script',
          children: scriptContent,
          injectTo: 'body',
        },
      ]
    },
  }
}
