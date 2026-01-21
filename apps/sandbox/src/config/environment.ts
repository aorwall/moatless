/**
 * Sandbox environment configuration
 */

import { SANDBOX_DATA_DIR, SANDBOX_WORKSPACE_DIR } from '@moatless/sandbox-schemas'
import { type RepositoryConfigFile, schemas } from '@moatless/types'
import { logger } from '../shared/logger'

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

  /**
   * Full repository config (from REPO_CONFIG env var)
   * Includes: systemPrompt, codeExecution (setupCommands, dockerImage), previewConfig
   * Used for auto-starting preview server at sandbox startup
   */
  repoConfig?: RepositoryConfigFile
}

/**
 * Parse repository config from REPO_CONFIG env var
 * Uses the same schema as .moatless/config.json
 */
function parseRepoConfig(): RepositoryConfigFile | undefined {
  const repoConfigStr = process.env['REPO_CONFIG']
  if (!repoConfigStr) {
    return undefined
  }

  try {
    const parsed = JSON.parse(repoConfigStr)
    const result = schemas.RepositoryConfigFileSchema.safeParse(parsed)

    if (!result.success) {
      logger.warn(
        { issues: result.error.issues },
        'Invalid REPO_CONFIG format, ignoring repository config',
      )
      return undefined
    }

    return result.data
  } catch (error) {
    logger.warn({ error }, 'Failed to parse REPO_CONFIG JSON, ignoring repository config')
    return undefined
  }
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
    repoConfig: parseRepoConfig(),
  }
}
