/**
 * Status types for preview server
 * Generic types with no dependencies on moatless-specific types
 */

export type ServerStatusType = 'stopped' | 'starting' | 'started' | 'failed'

export interface PreviewServerStatus {
  status: ServerStatusType
  pid?: number | undefined // Process ID
  internalUrl?: string | undefined // Internal server URL (e.g., 'http://localhost:3000') - for internal services only
  externalUrl?: string | undefined // External proxy URL (e.g., 'http://localhost:3001') - exposed to users
  uptimeSeconds?: number | undefined // Uptime in seconds
  currentPath?: string | undefined // Current navigation path
  error?: string | undefined // Error message (if failed)
  logs?: string | undefined // Server logs
  dirtyContext?: boolean | undefined // Whether workspace code has been modified since server start
}
