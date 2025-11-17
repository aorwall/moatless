import { ShellCommand } from '@moatless/shell-utils'
import type { ShellCommandLogEvent } from '@moatless/types'
import { injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { BaseAgent } from '../../agents/agent/base-agent'
import { ClaudeCodeAgent } from '../../claude/agent/claude-code-agent'
import type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  RunCommandOptions,
} from './base-environment-driver.js'

/**
 * LocalEnvironmentDriver - Manages local execution of commands and agents
 *
 * This driver:
 * - Is stateless regarding workspace lifecycle (state lives in snapshots)
 * - Manages active agent instances via map (keyed by taskId)
 * - Executes commands via ShellCommand
 *
 * Note: Preview server management moved to PreviewRuntimeService
 * Note: Setup commands now executed by PreviewServer as part of command sequence
 */
@injectable()
export class LocalEnvironmentDriver implements BaseEnvironmentDriver {
  /**
   * Active agent instances keyed by taskId
   * Agents spawn child processes (claude CLI) that need lifecycle management
   */
  private activeAgents: Map<string, BaseAgent> = new Map()

  /**
   * Run a command and stream log events
   */
  async *runCommand(
    workspacePath: string,
    command: string,
    options?: RunCommandOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    logger.info(
      {
        workspacePath,
        command,
        timeout: options?.timeout,
      },
      'Running command',
    )

    try {
      const shellCommand = new ShellCommand({
        workingDirectory: workspacePath,
        timeout: options?.timeout || 30000,
        ...(options?.env && { env: options.env }),
        ...(options?.logFile && { logFile: options.logFile }),
      })

      // execute() now returns a promise that resolves when process exits
      // We don't await it yet so we can stream logs concurrently
      const completionPromise = shellCommand.execute(command)

      // Stream logs from the shell command
      for await (const logEvent of shellCommand.streamLogs()) {
        yield logEvent
      }

      // Wait for process to complete
      await completionPromise

      logger.info(
        {
          command,
          exitCode: shellCommand.getExitCode() || 0,
          duration: shellCommand.getDuration(),
        },
        'Command completed',
      )
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)

      logger.error({ err: error as Error, command, errorMessage }, 'Command failed')

      // Emit error as system log
      yield {
        type: 'system',
        timestamp: new Date().toISOString(),
        text: `Error: ${errorMessage}\n`,
      }
    }
  }

  /**
   * Create an agent instance
   */
  createAgent(taskId: string, agentType: string, options?: AgentCreationOptions): BaseAgent {
    logger.debug(
      {
        taskId,
        agentType,
        model: options?.model,
        hasApiKey: !!options?.apiKey,
      },
      'Creating agent',
    )

    if (agentType !== 'claude-code') {
      throw new Error(`Unsupported agent type: ${agentType}`)
    }

    const agentOptions = {
      ...(options?.model && { model: options.model }),
      ...(options?.apiKey && { apiKey: options.apiKey }),
    }
    const agent = new ClaudeCodeAgent(taskId, agentOptions)
    this.activeAgents.set(taskId, agent)

    logger.info(
      {
        taskId,
        agentType,
        model: options?.model,
        hasApiKey: !!options?.apiKey,
      },
      'Agent created',
    )
    return agent
  }

  /**
   * Stop an agent instance
   */
  async stopAgent(taskId: string): Promise<void> {
    logger.debug({ taskId }, 'Stopping agent')

    const agent = this.activeAgents.get(taskId)
    if (!agent) {
      logger.debug({ taskId }, 'No active agent to stop')
      return
    }

    try {
      await agent.stop()
      this.activeAgents.delete(taskId)
      logger.info({ taskId }, 'Agent stopped')
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Error stopping agent')
      // Force cleanup even on error
      this.activeAgents.delete(taskId)
      throw error
    }
  }

  /**
   * Read a file from the workspace
   */
  async readFile(workspacePath: string, filePath: string): Promise<string> {
    const { promises: fs } = await import('node:fs')
    const path = await import('node:path')

    const fullPath = path.join(workspacePath, filePath)

    try {
      return await fs.readFile(fullPath, 'utf-8')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.error(
        { err: error as Error, workspacePath, filePath, errorMessage },
        'Failed to read file',
      )
      throw new Error(`Failed to read file ${filePath}: ${errorMessage}`)
    }
  }

  /**
   * Cleanup - stop all active agents
   * Note: Preview server cleanup is now handled by PreviewRuntimeService
   */
  async cleanup(): Promise<void> {
    logger.info(
      {
        agentCount: this.activeAgents.size,
      },
      'Cleaning up all resources',
    )

    // Stop all agents
    const agentTasks = Array.from(this.activeAgents.keys())
    await Promise.all(agentTasks.map((taskId) => this.stopAgent(taskId)))
    this.activeAgents.clear()

    logger.info('Cleanup completed')
  }
}
