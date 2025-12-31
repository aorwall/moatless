import type { GitChanges, GitStatus, ShellCommandLogEvent } from '@moatless/types'
import type { BaseAgent } from '../../agents/agent/base-agent'

/**
 * RunCommandOptions - Options for running commands
 */
export interface RunCommandOptions {
  timeout?: number
  env?: Record<string, string>
  logFile?: string
}

/**
 * AgentCreationOptions - Options for creating agents
 */
export interface AgentCreationOptions {
  model?: string
  apiKey?: string
  workspaceDir?: string
}

/**
 * FileSearchRequest - Options for searching files
 */
export interface FileSearchRequest {
  query: string
  limit?: number
}

/**
 * FileSearchResult - Result of a file search
 */
export interface FileSearchResult {
  files: Array<{ path: string; matches: number }>
  totalMatches: number
}

/**
 * PreviewServerConfig - Configuration for starting a preview server
 */
export interface PreviewServerConfig {
  serverCommand?: string
  envVars?: Record<string, string>
  setupCommands?: string[]
}

/**
 * PreviewServerStatus - Status of a preview server
 */
export interface PreviewServerStatus {
  status: 'stopped' | 'starting' | 'started' | 'failed'
  internalUrl?: string
  externalUrl?: string
  port?: number
  pid?: number
  uptimeSeconds?: number
  error?: string
  logs?: string
}

/**
 * BaseEnvironmentDriver - Stateless interface for environment operations
 *
 * Key principles:
 * - Drivers are stateless between invocations
 * - All persistent state lives in runtime snapshots
 * - Drivers implement shared interface for local and remote execution
 * - Managers orchestrate and update snapshots, drivers execute operations
 *
 * The driver provides a unified interface for:
 * - Command execution
 * - Agent lifecycle
 * - File operations
 * - Preview server management
 *
 * LocalEnvironmentDriver: Direct filesystem and process access
 * KubernetesEnvironmentDriver: HTTP forwarding to worker pod
 *
 * Note: The worker pod internally uses LocalEnvironmentDriver since
 * operations are local from the pod's perspective.
 */
export interface BaseEnvironmentDriver {
  // =============================================================================
  // Command Execution
  // =============================================================================

  /**
   * Run a command and stream log events
   * @param workspacePath - Path to workspace directory
   * @param command - Command to execute
   * @param options - Execution options (timeout, env vars, log file)
   */
  runCommand(
    workspacePath: string,
    command: string,
    options?: RunCommandOptions,
  ): AsyncGenerator<ShellCommandLogEvent>

  // =============================================================================
  // Agent Lifecycle
  // =============================================================================

  /**
   * Create an agent instance
   * @param taskId - Task ID for the agent
   * @param agentType - Type of agent to create (e.g., 'claude-code')
   * @param options - Agent creation options
   */
  createAgent(taskId: string, agentType: string, options?: AgentCreationOptions): BaseAgent

  /**
   * Stop an agent instance
   * @param taskId - Task ID for the agent to stop
   */
  stopAgent(taskId: string): Promise<void>

  // =============================================================================
  // File Operations
  // =============================================================================

  /**
   * Read a file from the workspace
   * @param workspacePath - Path to workspace directory
   * @param filePath - Relative path to file within workspace
   */
  readFile(workspacePath: string, filePath: string): Promise<string>

  /**
   * Write a file to the workspace
   * @param workspacePath - Path to workspace directory
   * @param filePath - Relative path to file within workspace
   * @param content - File content to write
   */
  writeFile(workspacePath: string, filePath: string, content: string): Promise<void>

  /**
   * List files in the workspace
   * @param workspacePath - Path to workspace directory
   * @param relativePath - Relative path within workspace to list
   * @param recursive - Whether to list recursively
   */
  listFiles(workspacePath: string, relativePath?: string, recursive?: boolean): Promise<string[]>

  /**
   * Search for files in the workspace
   * @param workspacePath - Path to workspace directory
   * @param request - Search options (query, limit)
   */
  searchFiles(workspacePath: string, request: FileSearchRequest): Promise<FileSearchResult>

  // =============================================================================
  // Git Operations
  // =============================================================================

  /**
   * Get git status for the workspace
   * @param workspacePath - Path to workspace directory
   */
  getGitStatus(workspacePath: string): Promise<GitStatus>

  /**
   * Get git changes for the workspace
   * @param workspacePath - Path to workspace directory
   */
  getGitChanges(workspacePath: string): Promise<GitChanges>

  // =============================================================================
  // Preview Server (Optional - for environments that support it)
  // =============================================================================

  /**
   * Start a preview server
   * @param taskId - Task ID for the preview server
   * @param config - Preview server configuration
   */
  startPreviewServer?(taskId: string, config: PreviewServerConfig): Promise<PreviewServerStatus>

  /**
   * Stop a preview server
   * @param taskId - Task ID for the preview server
   */
  stopPreviewServer?(taskId: string): Promise<boolean>

  /**
   * Get preview server status
   * @param taskId - Task ID for the preview server
   */
  getPreviewServerStatus?(taskId: string): Promise<PreviewServerStatus>

  /**
   * Get preview server logs
   * @param taskId - Task ID for the preview server
   * @param lines - Number of lines to return
   */
  getPreviewServerLogs?(taskId: string, lines?: number): Promise<string>
}
