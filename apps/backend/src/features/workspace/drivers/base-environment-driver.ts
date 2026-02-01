import type { AgentRunRequest } from '@moatless/sandbox-schemas'
import type {
  FileTreeWithChangesResponse,
  GitChanges,
  GitStatus,
  ShellCommandLogEvent,
} from '@moatless/types'

/**
 * RunCommandOptions - Options for running commands
 */
export interface RunCommandOptions {
  timeout?: number
  env?: Record<string, string>
  logFile?: string
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
 * PreviewServerStatus - Status of a preview server
 */
export interface PreviewServerStatus {
  status: 'stopped' | 'installing' | 'starting' | 'started' | 'failed'
  internalUrl?: string
  externalUrl?: string
  port?: number
  pid?: number
  uptimeSeconds?: number
  error?: string
  logs?: string
}

export interface PreviewServerLogEvent {
  type: string
  data: {
    text?: string
    error?: string
    timestamp: string
  }
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
 * - WorkspaceProvisioner creates driver instances per task
 *
 * The driver provides a unified interface for:
 * - Command execution
 * - Agent lifecycle
 * - File operations
 * - Preview server management
 *
 * Implementation: RemoteEnvironmentDriver - HTTP forwarding to worker pods/containers
 *
 * Note: The worker pod/container internally handles operations locally.
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
   * Execute an agent with the given request.
   * This is a fire-and-forget HTTP POST - events are handled via Redis subscription.
   * @param request - Agent run request containing content and options
   */
  executeAgent(request: AgentRunRequest): Promise<void>

  /**
   * Stop the agent instance for this task
   */
  stopAgent(): Promise<void>

  /**
   * Notify the sandbox that new messages are available for the agent.
   * This is a best-effort push notification to wake up the message polling consumer.
   * The sandbox will fetch the actual messages from the backend API.
   */
  notifyMessages(): Promise<void>

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
   * Get the external preview URL accessible by the user's browser
   */
  getExternalPreviewUrl?(): string | undefined

  /**
   * Get the worker base URL for proxying requests to the sandbox
   */
  getWorkerBaseUrl?(): Promise<string | undefined>

  /**
   * Get preview server status
   */
  getPreviewServerStatus?(): Promise<PreviewServerStatus>

  /**
   * Get preview server logs
   * @param lines - Number of lines to return
   */
  getPreviewServerLogs?(lines?: number): Promise<string>

  /**
   * Stream preview server logs
   * @param lines - Number of lines to return for non-streaming backends
   */
  streamPreviewServerLogs?(lines?: number): AsyncGenerator<PreviewServerLogEvent>
}
