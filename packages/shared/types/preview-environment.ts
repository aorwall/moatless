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
  portPlaceholder?: string
  // Used to validate the process is still the same one we started
  processStartTime?: number
  // Proxy configuration if using proxy injection
  proxyPort?: number
  proxyUrl?: string
  // Environment variables used to start the server
  envVars?: Record<string, string>
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
