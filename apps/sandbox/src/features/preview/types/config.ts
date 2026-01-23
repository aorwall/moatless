/**
 * Configuration types for preview server
 * These are minimal, generic types with no dependencies on moatless-specific types
 */

/** Configuration for starting a preview server */
export interface PreviewServerConfig {
  // Identity
  serverId: string // For logging/tracking
  workspacePath: string // Working directory

  // Server startup
  setupCommands?: string[] | undefined // Commands to run before server (e.g., ['npm', 'install'])
  serverCommands: string[] // Commands to run (e.g., ['npm', 'start'])
  envVars?: Record<string, string> | undefined // Environment variables

  // Behavior
  startupTimeoutSeconds?: number | undefined // HTTP ready timeout (default: 90)
  enableInspector?: boolean | undefined // Inject inspector script (default: true)
}

/** Configuration for hostname-based routing proxy */
export interface HostnameProxyConfig {
  proxyPort: number // Port to listen on
  proxyDomain: string // Base domain (e.g., 'proxy.moatless.xyz')
  protocol: 'http' | 'https' // Protocol
}
