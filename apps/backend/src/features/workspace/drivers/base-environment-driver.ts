import type { AgentRunRequest } from '@moatless/sandbox-schemas'
import type { ShellCommandLogEvent } from '@moatless/types'

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
 * - File search
 * - Preview server management
 *
 * Implementation: RemoteEnvironmentDriver - HTTP forwarding to sandbox pods/containers
 *
 * Note: The sandbox pod/container internally handles operations locally.
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
   * Search for files in the workspace
   * @param request - Search options (query, limit)
   */
  searchFiles(request: FileSearchRequest): Promise<FileSearchResult>

  // =============================================================================
  // Preview Server (Optional - for environments that support it)
  // =============================================================================

  /**
   * Get the external preview URL accessible by the user's browser
   */
  getExternalPreviewUrl?(): string | undefined

  /**
   * Get the sandbox base URL for proxying requests to the sandbox
   */
  getSandboxBaseUrl?(): Promise<string | undefined>

  /**
   * Get preview server status
   */
  getPreviewServerStatus?(): Promise<PreviewServerStatus>

  /**
   * Get status for a specific server by name
   * @param serverName - Server name (e.g., 'preview', 'storybook')
   */
  getServerStatus?(serverName: string): Promise<PreviewServerStatus>

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

  // =============================================================================
  // Multi-Server Logs (Generic - for any server by name)
  // =============================================================================

  /**
   * Get logs for a specific server by name
   * @param serverName - Server name (e.g., 'preview', 'storybook')
   * @param lines - Number of lines to return
   */
  getServerLogs?(serverName: string, lines?: number): Promise<string>

  /**
   * Stream logs for a specific server by name
   * @param serverName - Server name (e.g., 'preview', 'storybook')
   * @param lines - Number of lines to return for non-streaming backends
   */
  streamServerLogs?(serverName: string, lines?: number): AsyncGenerator<PreviewServerLogEvent>
}
