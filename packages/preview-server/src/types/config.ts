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
  serverCommands: string[] // Commands to run (e.g., ['npm', 'start'])
  envVars?: Record<string, string> | undefined // Environment variables

  // Behavior
  startupTimeoutSeconds?: number | undefined // HTTP ready timeout (default: 30)
  enableInspector?: boolean | undefined // Inject inspector script (default: true)

  // Optional
  logFile?: string | undefined // Log file path
}

/** Configuration for per-task proxy */
export interface ProxyConfig {
  targetUrl: string // Target server URL
  proxyPort: number // Proxy port
  scriptContent?: string | undefined // Optional script to inject
  serverId?: string | undefined // For logging
  logger?: unknown | undefined // Logger instance (uses noop if not provided)
}

/** Configuration for hostname-based routing proxy */
export interface HostnameProxyConfig {
  proxyPort: number // Port to listen on
  proxyDomain: string // Base domain (e.g., 'proxy.moatless.ai')
  protocol: 'http' | 'https' // Protocol
}
