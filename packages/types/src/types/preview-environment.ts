/**
 * Preview environment server metadata that is persisted to disk
 */
export interface PreviewEnvironmentMetadata {
  taskId: string
  pid: number
  port: number
  startTime: Date
  command: string
  workspacePath: string
  logFile: string
  // Used to validate the process is still the same one we started
  processStartTime?: number
  // Environment variables used to start the server
  envVars?: Record<string, string>
  // Error information for failed startups
  error?: string
  logs?: string
  failed?: boolean
  // Indicates if process is running but hasn't bound to port yet
  starting?: boolean
  // Proxy server information for SPA support
  proxyPort?: number
  proxyUrl?: string
}

/**
 * Options for preview environment server operations
 */
export interface PreviewEnvironmentOptions {
  timeout?: number
  forceKill?: boolean
  validateProcess?: boolean
}

/**
 * Process validation result
 */
export interface ProcessValidation {
  exists: boolean
  isCorrectProcess: boolean
  command?: string
  startTime?: number
}
