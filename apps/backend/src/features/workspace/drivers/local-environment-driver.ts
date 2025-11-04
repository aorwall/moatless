import { ShellCommand, type ShellCommandOptions } from '@moatless/shell-utils'
import type { ShellCommandLogEvent } from '@moatless/types'
import { injectable } from 'inversify'
import { logger } from '../../../utils/logger.js'
import { ensureTaskLogDirectory, getTaskLogPath } from '../../../utils/task-log.js'
import type { BaseAgent } from '../../agents/implementations/base-agent.js'
import { ClaudeCodeAgent } from '../../agents/implementations/claude-code/claude-code-agent.js'
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
 * - Provides unified log paths for setup + preview commands
 *
 * Note: Preview server management moved to PreviewRuntimeService
 */
@injectable()
export class LocalEnvironmentDriver implements BaseEnvironmentDriver {
  /**
   * Active agent instances keyed by taskId
   * Agents spawn child processes (claude CLI) that need lifecycle management
   */
  private activeAgents: Map<string, BaseAgent> = new Map()

  /**
   * Execute workspace setup commands
   * Prepares workspace environment with dependencies (e.g., npm install, bun install)
   * Writes logs to unified task log file for streaming
   */
  async executeSetupCommands(
    taskId: string,
    workspacePath: string,
    commands: string[],
    envVars?: Record<string, string>,
  ): Promise<void> {
    if (commands.length === 0) {
      logger.debug('LocalEnvironmentDriver: No setup commands to execute')
      return
    }

    // Get unified log path for this task
    const logFile = await this.ensureTaskLog(taskId)

    logger.info('LocalEnvironmentDriver: Executing setup commands', {
      workspacePath,
      commands,
      logFile,
    })

    for (const command of commands) {
      logger.info(`LocalEnvironmentDriver: Running setup command: ${command}`)

      const shellCommand = new ShellCommand()
      const result = await shellCommand.executeWithResult(command, {
        workingDirectory: workspacePath,
        timeout: 300000, // 5 minutes for installs
        logFile, // Write to unified task log file
        ...(envVars !== undefined && { env: envVars }),
      })

      if (result.exitCode !== 0) {
        const error = `Setup command failed: ${command}. Exit code: ${result.exitCode}. Output: ${result.output}`
        logger.error('LocalEnvironmentDriver: Setup command failed', new Error(error), {
          command,
          exitCode: result.exitCode,
        })
        throw new Error(error)
      }

      logger.info('LocalEnvironmentDriver: Setup command completed successfully', {
        command,
        exitCode: result.exitCode,
        durationMs: result.durationMs,
      })
    }

    logger.info('LocalEnvironmentDriver: All setup commands completed')
  }

  /**
   * Ensure task log file exists and return its path
   * Used by preview orchestrator to get unified log path for setup + preview
   */
  async ensureTaskLog(taskId: string): Promise<string> {
    await ensureTaskLogDirectory()
    return getTaskLogPath(taskId)
  }

  /**
   * Run a command and stream log events
   */
  async *runCommand(
    workspacePath: string,
    command: string,
    options?: RunCommandOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    logger.info('LocalEnvironmentDriver: Running command', {
      workspacePath,
      command,
      timeout: options?.timeout,
    })

    try {
      const shellCommand = new ShellCommand()

      const shellOptions: ShellCommandOptions = {
        workingDirectory: workspacePath,
        timeout: options?.timeout || 30000,
        background: true, // Run in background for streaming
      }

      if (options?.env) {
        shellOptions.env = options.env
      }

      if (options?.logFile) {
        shellOptions.logFile = options.logFile
      }

      await shellCommand.execute(command, shellOptions)

      // Stream logs from the shell command
      for await (const logEvent of shellCommand.streamLogs()) {
        yield logEvent
      }

      logger.info('LocalEnvironmentDriver: Command completed', {
        command,
        exitCode: shellCommand.getExitCode() || 0,
        duration: shellCommand.getDuration(),
      })
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)

      logger.error('LocalEnvironmentDriver: Command failed', error as Error, {
        command,
        errorMessage,
      })

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
    logger.info('LocalEnvironmentDriver: Creating agent', {
      taskId,
      agentType,
      model: options?.model,
    })

    if (agentType !== 'claude-code') {
      throw new Error(`Unsupported agent type: ${agentType}`)
    }

    const agentOptions = options?.model ? { model: options.model } : {}
    const agent = new ClaudeCodeAgent(taskId, agentOptions)
    this.activeAgents.set(taskId, agent)

    logger.info('LocalEnvironmentDriver: Agent created and stored', { taskId, agentType })
    return agent
  }

  /**
   * Stop an agent instance
   */
  async stopAgent(taskId: string): Promise<void> {
    logger.info('LocalEnvironmentDriver: Stopping agent', { taskId })

    const agent = this.activeAgents.get(taskId)
    if (!agent) {
      logger.debug('LocalEnvironmentDriver: No active agent to stop', { taskId })
      return
    }

    try {
      await agent.stop()
      this.activeAgents.delete(taskId)
      logger.info('LocalEnvironmentDriver: Agent stopped', { taskId })
    } catch (error) {
      logger.error('LocalEnvironmentDriver: Error stopping agent', error as Error, { taskId })
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
      logger.error('LocalEnvironmentDriver: Failed to read file', error as Error, {
        workspacePath,
        filePath,
        errorMessage,
      })
      throw new Error(`Failed to read file ${filePath}: ${errorMessage}`)
    }
  }

  /**
   * Cleanup - stop all active agents
   * Note: Preview server cleanup is now handled by PreviewRuntimeService
   */
  async cleanup(): Promise<void> {
    logger.info('LocalEnvironmentDriver: Cleaning up all resources', {
      agentCount: this.activeAgents.size,
    })

    // Stop all agents
    const agentTasks = Array.from(this.activeAgents.keys())
    await Promise.all(agentTasks.map((taskId) => this.stopAgent(taskId)))
    this.activeAgents.clear()

    logger.info('LocalEnvironmentDriver: Cleanup completed')
  }
}
