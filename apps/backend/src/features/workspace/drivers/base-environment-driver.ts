import type { ServerStatus, ShellCommandLogEvent } from '@moatless/types'
import type { BaseAgent } from '../../../agents/base-agent'

/**
 * RunCommandOptions - Options for running commands
 */
export interface RunCommandOptions {
  timeout?: number
  env?: Record<string, string>
  logFile?: string
}

/**
 * PreviewConfiguration - Configuration for starting preview servers
 */
export interface PreviewConfiguration {
  taskId: string
  workspacePath: string
  serverStartCommands: string[]
  envVars?: Record<string, string> | undefined
  portRange?: { min: number; max: number } | undefined
  logFile?: string | undefined
}

/**
 * AgentCreationOptions - Options for creating agents
 */
export interface AgentCreationOptions {
  model?: string
  workspaceDir?: string
}

/**
 * BaseEnvironmentDriver - Stateless interface for environment operations
 *
 * Key principles:
 * - Drivers are stateless between invocations
 * - All persistent state lives in runtime snapshots
 * - Drivers implement shared interface for local and remote execution
 * - Managers orchestrate and update snapshots, drivers execute operations
 */
export interface BaseEnvironmentDriver {
  /**
   * Execute workspace setup commands
   * Prepares workspace environment with dependencies (e.g., npm install, bun install)
   * @param workspacePath - Path to workspace directory
   * @param commands - Setup commands to execute
   * @param envVars - Environment variables for command execution
   */
  executeSetupCommands(
    workspacePath: string,
    commands: string[],
    envVars?: Record<string, string>,
  ): Promise<void>

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

  /**
   * Start a preview server
   * @param config - Preview server configuration
   * @returns Server status including URL and port
   */
  startPreview(config: PreviewConfiguration): Promise<ServerStatus>

  /**
   * Stop a preview server
   * @param taskId - Task ID for the preview server
   */
  stopPreview(taskId: string): Promise<void>

  /**
   * Get preview server status
   * @param taskId - Task ID for the preview server
   */
  getPreviewStatus(taskId: string): Promise<ServerStatus>

  /**
   * Get preview server logs
   * @param taskId - Task ID for the preview server
   * @param lines - Number of lines to retrieve
   */
  getPreviewLogs(taskId: string, lines?: number): Promise<string>

  /**
   * Stream preview server logs
   * @param taskId - Task ID for the preview server
   */
  streamPreviewLogs(taskId: string): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }>

  /**
   * Proxy a request to the preview server
   * @param taskId - Task ID for the preview server
   * @param request - HTTP request details
   */
  proxyPreviewRequest(
    taskId: string,
    request: {
      method: string
      path: string
      headers: Headers
      body?: string
    },
  ): Promise<{
    status: number
    headers: Headers
    body: string | Buffer
  }>

  /**
   * Get the port number for the preview server
   * @param taskId - Task ID for the preview server
   */
  getPreviewPort(taskId: string): number | undefined

  /**
   * Create an agent instance
   * @param taskId - Task ID for the agent
   * @param agentType - Type of agent to create (e.g., 'claude-code')
   * @param options - Agent creation options
   */
  createAgent(taskId: string, agentType: string, options?: AgentCreationOptions): BaseAgent

  /**
   * Read a file from the workspace
   * @param workspacePath - Path to workspace directory
   * @param filePath - Relative path to file within workspace
   */
  readFile(workspacePath: string, filePath: string): Promise<string>
}
