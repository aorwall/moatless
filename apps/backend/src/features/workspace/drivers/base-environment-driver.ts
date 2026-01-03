import type {
  FileTreeWithChangesResponse,
  GitChanges,
  GitStatus,
  ShellCommandLogEvent,
} from '@moatless/types'
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
 * GitDiffResult - Result of getting git diff
 */
export interface GitDiffResult {
  diff?: string
  files: Array<{ oldFile?: string; newFile?: string; hunks: string[] }>
}

/**
 * FileWithChangesResult - Result of reading a file with change information
 */
export interface FileWithChangesResult {
  content: string
  oldContent?: string
  additions?: number
  deletions?: number
  isModified?: boolean
}

/**
 * BaseEnvironmentDriver - Task-scoped interface for environment operations
 *
 * Key principles:
 * - Each driver instance is scoped to a single task
 * - taskId and workspace context are set at construction time
 * - Methods don't need workspacePath - it's an instance variable
 * - FileGateway creates/caches driver instances per task
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
  /**
   * The task ID this driver instance is scoped to
   */
  readonly taskId: string

  // =============================================================================
  // Command Execution
  // =============================================================================

  /**
   * Run a command and stream log events
   * @param command - Command to execute
   * @param options - Execution options (timeout, env vars, log file)
   */
  runCommand(command: string, options?: RunCommandOptions): AsyncGenerator<ShellCommandLogEvent>

  // =============================================================================
  // Agent Lifecycle
  // =============================================================================

  /**
   * Create an agent instance
   * @param agentType - Type of agent to create (e.g., 'claude-code')
   * @param options - Agent creation options
   */
  createAgent(agentType: string, options?: AgentCreationOptions): Promise<BaseAgent>

  /**
   * Stop the agent instance for this task
   */
  stopAgent(): Promise<void>

  // =============================================================================
  // File Operations
  // =============================================================================

  /**
   * Read a file from the workspace
   * @param filePath - Relative path to file within workspace
   */
  readFile(filePath: string): Promise<string>

  /**
   * Write a file to the workspace
   * @param filePath - Relative path to file within workspace
   * @param content - File content to write
   */
  writeFile(filePath: string, content: string): Promise<void>

  /**
   * List files in the workspace
   * @param relativePath - Relative path within workspace to list
   * @param recursive - Whether to list recursively
   */
  listFiles(relativePath?: string, recursive?: boolean): Promise<string[]>

  /**
   * Search for files in the workspace
   * @param request - Search options (query, limit)
   */
  searchFiles(request: FileSearchRequest): Promise<FileSearchResult>

  // =============================================================================
  // Git Operations
  // =============================================================================

  /**
   * Get git status for the workspace
   */
  getGitStatus(): Promise<GitStatus>

  /**
   * Get git changes for the workspace
   */
  getGitChanges(): Promise<GitChanges>

  /**
   * Push changes to remote repository
   * @param options - Push options (branch, useUpstream flag)
   * @returns Push result with success status and any error message
   */
  pushChanges(options: {
    branch: string
    useUpstream?: boolean
  }): Promise<{ pushed: boolean; hasCommits: boolean; error?: string }>

  /**
   * Create a git checkpoint (commit) for uncommitted changes
   * @param message - Commit message for the checkpoint
   * @returns Checkpoint result with commit hash if successful
   */
  createGitCheckpoint(
    message: string,
  ): Promise<{ commitHash: string | null; hasChanges: boolean; error?: string }>

  // =============================================================================
  // Extended Git Operations (for file tree and diff views)
  // =============================================================================

  /**
   * Get file tree with git change information
   * Combines file listing with git diff data for tree views
   * @param rootPath - Root path to start listing from
   */
  getFileTreeWithChanges(rootPath?: string): Promise<FileTreeWithChangesResponse>

  /**
   * Get git diff for the workspace
   * @returns Diff string and file change information
   */
  getGitDiff(): Promise<GitDiffResult>

  /**
   * Read a file with git change information
   * @param filePath - Relative path to file within workspace
   * @returns File content with optional old content and change stats
   */
  readFileWithChanges(filePath: string): Promise<FileWithChangesResult>

  /**
   * Get the current commit hash
   * @returns Short commit hash or null if not in a git repo
   */
  getCurrentCommitHash(): Promise<string | null>

  // =============================================================================
  // Preview Server (Optional - for environments that support it)
  // =============================================================================

  /**
   * Start a preview server
   * @param config - Preview server configuration
   */
  startPreviewServer?(config: PreviewServerConfig): Promise<PreviewServerStatus>

  /**
   * Stop a preview server
   */
  stopPreviewServer?(): Promise<boolean>

  /**
   * Get preview server status
   */
  getPreviewServerStatus?(): Promise<PreviewServerStatus>

  /**
   * Get preview server logs
   * @param lines - Number of lines to return
   */
  getPreviewServerLogs?(lines?: number): Promise<string>
}
