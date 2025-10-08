import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * Generate the full preview inspector script that gets injected into preview iframe
 * This script provides console logging, network monitoring, element inspection, and more
 */
export function getPreviewInspectorScript(serverId: string): string {
  // Get the directory of this file
  const __filename = fileURLToPath(import.meta.url)
  const __dirname = path.dirname(__filename)

  // Read the JavaScript file from scripts directory
  const scriptPath = path.join(__dirname, '..', 'scripts', 'preview-inspector.js')
  let scriptContent = readFileSync(scriptPath, 'utf-8')

  // Replace the placeholder with the actual serverId
  scriptContent = scriptContent.replace(/__SERVER_ID__/g, serverId)

  return scriptContent
}
