import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export interface InspectorScriptOptions {
  /** Server ID to identify this preview instance in postMessage communication */
  serverId?: string
}

/**
 * Get the bundled inspector client script with serverId substituted.
 * Used for proxy-based injection (sandbox use case).
 */
export function getInspectorScript(options: InspectorScriptOptions = {}): string {
  const __filename = fileURLToPath(import.meta.url)
  const __dirname = path.dirname(__filename)

  const scriptPath = path.join(__dirname, 'client.js')
  let scriptContent = readFileSync(scriptPath, 'utf-8')

  if (options.serverId) {
    scriptContent = scriptContent.replace(/__SERVER_ID__/g, options.serverId)
  }

  return scriptContent
}
