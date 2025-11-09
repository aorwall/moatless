/**
 * Status types for preview server
 * Generic types with no dependencies on moatless-specific types
 */

export type ServerStatusType = 'stopped' | 'starting' | 'started' | 'failed'

export interface PreviewServerStatus {
  status: ServerStatusType
  pid?: number | undefined // Process ID
  url?: string | undefined // Server URL (e.g., 'http://localhost:3000')
  proxyUrl?: string | undefined // Proxy URL (if enabled)
  uptimeSeconds?: number | undefined // Uptime in seconds
  currentPath?: string | undefined // Current navigation path
  error?: string | undefined // Error message (if failed)
  logs?: string | undefined // Server logs
  dirtyContext?: boolean | undefined // Whether workspace code has been modified since server start
}
