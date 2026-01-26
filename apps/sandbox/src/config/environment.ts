/**
 * Sandbox environment configuration
 */

import { SANDBOX_DATA_DIR, SANDBOX_WORKSPACE_DIR } from '@moatless/sandbox-schemas'
import { type RepositoryConfigFile, schemas } from '@moatless/types'
import { logger } from '../shared/logger'

/**
 * Agent configuration parsed from REPO_CONFIG.agentConfig
 * These are static settings that apply to all requests for this task
 */
export interface SandboxAgentConfig {
  /**
   * Plugin identifiers to enable (format: {plugin-name}@{marketplace-name})
   */
  plugins?: string[]

  /**
   * Default execution timeout in milliseconds
   */
  timeout?: number

  /**
   * Default model for agent execution
   */
  model?: string

  /**
   * System prompt to append to Claude Code default prompt
   * (from REPO_CONFIG.systemPrompt)
   */
  systemPrompt?: string
}

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
   * Includes: systemPrompt, codeExecution (setupCommands, dockerImage), previewConfig, agentConfig
   * Used for auto-starting preview server at sandbox startup
   */
  repoConfig?: RepositoryConfigFile

  /**
   * Agent configuration parsed from REPO_CONFIG.agentConfig + systemPrompt
   * These are static settings that apply to all requests for this task
   */
  agentConfig: SandboxAgentConfig
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
 * Parse agent configuration from REPO_CONFIG
 * Combines agentConfig fields with systemPrompt for convenience
 */
function parseAgentConfig(repoConfig: RepositoryConfigFile | undefined): SandboxAgentConfig {
  return {
    plugins: repoConfig?.agentConfig?.plugins,
    timeout: repoConfig?.agentConfig?.timeout,
    model: repoConfig?.agentConfig?.model,
    systemPrompt: repoConfig?.systemPrompt,
  }
}

// Singleton config instance - initialized once at startup
let _config: SandboxConfig | null = null

/**
 * Load configuration from environment variables
 * This function is idempotent - subsequent calls return the cached config
 */
export function loadConfig(): SandboxConfig {
  // Return cached config if already loaded
  if (_config) {
    return _config
  }

  const taskId = process.env['TASK_ID']
  if (!taskId) {
    throw new Error('TASK_ID environment variable is required')
  }

  const repoConfig = parseRepoConfig()
  const agentConfig = parseAgentConfig(repoConfig)

  // Log agent config for debugging
  if ((agentConfig.plugins?.length ?? 0) > 0 || agentConfig.timeout || agentConfig.model) {
    logger.info(
      {
        plugins: agentConfig.plugins,
        timeout: agentConfig.timeout,
        model: agentConfig.model,
        hasSystemPrompt: !!agentConfig.systemPrompt,
      },
      'Agent config loaded from REPO_CONFIG',
    )
  }

  _config = {
    port: Number.parseInt(process.env['PORT'] || '8001', 10),
    taskId,
    workspaceDir: process.env['WORKSPACE_DIR'] || SANDBOX_WORKSPACE_DIR,
    dataDir: process.env['DATA_DIR'] || SANDBOX_DATA_DIR,
    uploadDir: process.env['UPLOAD_DIR'] || `${SANDBOX_DATA_DIR}/uploads`,
    databaseUrl: process.env['DATABASE_URL'],
    logLevel: process.env['LOG_LEVEL'] || 'info',
    gitCloneUrl: process.env['GIT_CLONE_URL'],
    gitBranch: process.env['GIT_BRANCH'],
    repoConfig,
    agentConfig,
  }

  return _config
}

/**
 * Get the agent config from the loaded sandbox config
 * This can be called from anywhere after loadConfig() has been called
 */
export function getAgentConfig(): SandboxAgentConfig {
  if (!_config) {
    throw new Error('Config not loaded. Call loadConfig() first.')
  }
  return _config.agentConfig
}
