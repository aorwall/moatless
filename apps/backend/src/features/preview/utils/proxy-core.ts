/**
 * Extract the subdomain from a hostname given the base proxy domain.
 *
 * Example: extractSubdomain('task-abc.proxy.example.com', 'proxy.example.com') => 'task-abc'
 */
export function extractSubdomain(hostname: string, proxyDomain: string): string | null {
  const suffix = `.${proxyDomain}`
  if (!hostname.endsWith(suffix)) {
    return null
  }
  const subdomain = hostname.slice(0, -suffix.length)
  return subdomain || null
}

/**
 * Parse a subdomain into taskId and optional serverName.
 * Uses '--' as separator between taskId and serverName.
 *
 * Examples:
 *   'task-123'          → { taskId: 'task-123', serverName: undefined }
 *   'task-123--files'   → { taskId: 'task-123', serverName: 'files' }
 *   'task-123--preview' → { taskId: 'task-123', serverName: 'preview' }
 */
export function parseSubdomain(subdomain: string): { taskId: string; serverName?: string } {
  const separatorIndex = subdomain.indexOf('--')
  if (separatorIndex === -1) {
    return { taskId: subdomain }
  }
  return {
    taskId: subdomain.slice(0, separatorIndex),
    serverName: subdomain.slice(separatorIndex + 2),
  }
}
