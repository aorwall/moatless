import type { ShellCommandLogEvent } from '@moatless/types'
import type { BaseAgent } from '../../agents/implementations/base-agent'

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
 * BaseEnvironmentDriver - Stateless interface for environment operations
 *
 * Key principles:
 * - Drivers are stateless between invocations
 * - All persistent state lives in runtime snapshots
 * - Drivers implement shared interface for local and remote execution
 * - Managers orchestrate and update snapshots, drivers execute operations
 *
 * Note: Setup commands now handled by PreviewServer as part of command sequence
 */
export interface BaseEnvironmentDriver {
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

  /**
   * Read a file from the workspace
   * @param workspacePath - Path to workspace directory
   * @param filePath - Relative path to file within workspace
   */
  readFile(workspacePath: string, filePath: string): Promise<string>
}
