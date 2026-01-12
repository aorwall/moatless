/**
 * Sandbox environment configuration
 */

import { SANDBOX_DATA_DIR, SANDBOX_WORKSPACE_DIR } from '@moatless/sandbox-schemas'

export interface SandboxConfig {
  /**
   * Port to listen on
   */
  port: number

  /**
   * Task ID this sandbox is scoped to
   */
  taskId: string

  /**
   * Workspace directory for file operations
   */
  workspaceDir: string

  /**
   * Data directory for storage
   */
  dataDir: string

  /**
   * Upload directory for file attachments
   */
  uploadDir: string

  /**
   * Database URL for event storage (PostgreSQL)
   */
  databaseUrl?: string

  /**
   * Log level
   */
  logLevel: string

  /**
   * Git repository URL to clone on startup
   */
  gitCloneUrl?: string

  /**
   * Git branch to checkout
   */
  gitBranch?: string
}

/**
 * Load configuration from environment variables
 */
export function loadConfig(): SandboxConfig {
  const taskId = process.env['WORKER_TASK_ID']
  if (!taskId) {
    throw new Error('WORKER_TASK_ID environment variable is required')
  }

  return {
    port: Number.parseInt(process.env['PORT'] || '8001', 10),
    taskId,
    workspaceDir: process.env['WORKSPACE_DIR'] || SANDBOX_WORKSPACE_DIR,
    dataDir: process.env['DATA_DIR'] || SANDBOX_DATA_DIR,
    uploadDir: process.env['UPLOAD_DIR'] || `${SANDBOX_DATA_DIR}/uploads`,
    databaseUrl: process.env['DATABASE_URL'],
    logLevel: process.env['LOG_LEVEL'] || 'info',
    gitCloneUrl: process.env['GIT_CLONE_URL'],
    gitBranch: process.env['GIT_BRANCH'],
  }
}
