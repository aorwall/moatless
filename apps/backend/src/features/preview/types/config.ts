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

/** Configuration for per-task proxy */
export interface ProxyConfig {
  targetUrl: string // Target server URL
  proxyPort: number // Proxy port
  scriptContent?: string | undefined // Optional script to inject
  serverId?: string | undefined // For logging
  serverHost?: string | undefined // Host where preview servers run (default: localhost)
  // Note: Logger is now provided by pino (not passed in config)
}
