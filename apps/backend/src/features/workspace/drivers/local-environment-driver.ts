import { PreviewServer, type PreviewServerLifecycleSink } from '@moatless/preview-server'
import { ShellCommand, type ShellCommandOptions } from '@moatless/shell-utils'
import type { ServerStatus, ShellCommandLogEvent } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { toServerStatus } from '../../../adapters/preview-adapter'
import type { BaseAgent } from '../../../agents/base-agent'
import { ClaudeCodeAgent } from '../../../agents/claude-code/claude-code-agent'
import { PreviewServerLifecycleSinkSymbol } from '../../../container/identifiers'
import { logger } from '../../../utils/logger'
import type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  PreviewConfiguration,
  RunCommandOptions,
} from './base-environment-driver'

/**
 * LocalEnvironmentDriver - Manages local execution of commands, preview servers, and agents
 *
 * This driver:
 * - Is stateless regarding workspace lifecycle (state lives in snapshots)
 * - Manages active preview servers via map (keyed by taskId)
 * - Creates agent instances on demand
 * - Executes commands via ShellCommand
 */
@injectable()
export class LocalEnvironmentDriver implements BaseEnvironmentDriver {
  /**
   * Active preview servers keyed by taskId
   * Note: This is the only state the driver maintains
   */
  private previewServers: Map<string, PreviewServer> = new Map()

  constructor(
    @inject(PreviewServerLifecycleSinkSymbol)
    private readonly lifecycleSink: PreviewServerLifecycleSink,
  ) {}

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
   * Start a preview server
   */
  async startPreview(config: PreviewConfiguration): Promise<ServerStatus> {
    logger.info('LocalEnvironmentDriver: Starting preview server', {
      taskId: config.taskId,
      workspacePath: config.workspacePath,
    })

    // Check if server already exists
    const existingServer = this.previewServers.get(config.taskId)
    if (existingServer) {
      const status = existingServer.getStatus()
      if (status.status === 'started') {
        logger.info('LocalEnvironmentDriver: Server already running', {
          taskId: config.taskId,
          url: status.url,
        })
        return toServerStatus(status)
      }
    }

    try {
      // Create preview server config
      const previewConfig = {
        serverId: config.taskId,
        workspacePath: config.workspacePath,
        serverCommands: config.serverStartCommands, // PreviewServerConfig uses 'serverCommands'
        envVars: config.envVars || {},
        portRange: config.portRange || { min: 3000, max: 9000 },
        logFile: config.logFile,
      }

      // Create and start preview server
      const server = new PreviewServer(previewConfig, logger, this.lifecycleSink)
      this.previewServers.set(config.taskId, server)

      const result = await server.start()
      const status = toServerStatus(result)

      logger.info('LocalEnvironmentDriver: Preview server started', {
        taskId: config.taskId,
        status: status.status,
        url: status.url,
      })

      return status
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.error('LocalEnvironmentDriver: Failed to start preview server', error as Error, {
        taskId: config.taskId,
        errorMessage,
      })

      return {
        status: 'failed',
        uptimeSeconds: 0,
        error: errorMessage,
      }
    }
  }

  /**
   * Stop a preview server
   */
  async stopPreview(taskId: string): Promise<void> {
    logger.info('LocalEnvironmentDriver: Stopping preview server', { taskId })

    const server = this.previewServers.get(taskId)
    if (!server) {
      logger.debug('LocalEnvironmentDriver: No server to stop', { taskId })
      return
    }

    try {
      await server.stop()
      this.previewServers.delete(taskId)
      logger.info('LocalEnvironmentDriver: Preview server stopped', { taskId })
    } catch (error) {
      logger.error('LocalEnvironmentDriver: Error stopping preview server', error as Error, {
        taskId,
      })
      // Force cleanup
      this.previewServers.delete(taskId)
      throw error
    }
  }

  /**
   * Get preview server status
   */
  // biome-ignore lint/suspicious/useAwait: Must match interface signature
  async getPreviewStatus(taskId: string): Promise<ServerStatus> {
    const server = this.previewServers.get(taskId)
    if (!server) {
      return {
        status: 'stopped',
        pid: undefined,
        url: undefined,
        uptimeSeconds: 0,
      }
    }

    const status = server.getStatus()
    return toServerStatus(status)
  }

  /**
   * Get preview server logs
   */
  async getPreviewLogs(taskId: string, lines = 100): Promise<string> {
    const server = this.previewServers.get(taskId)
    if (!server) {
      return 'No preview server running'
    }

    return await server.getLogs(lines)
  }

  /**
   * Stream preview server logs
   */
  // biome-ignore lint/suspicious/useAwait: Generator function needs async for yield*
  async *streamPreviewLogs(taskId: string): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }> {
    const server = this.previewServers.get(taskId)
    if (!server) {
      yield {
        type: 'error',
        data: {
          error: 'No preview server running',
          timestamp: new Date().toISOString(),
        },
      }
      return
    }

    yield* server.streamLogs()
  }

  /**
   * Proxy a request to the preview server
   */
  async proxyPreviewRequest(
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
  }> {
    const server = this.previewServers.get(taskId)
    if (!server) {
      throw new Error(`No preview server running for task: ${taskId}`)
    }

    return await server.proxyRequest(request)
  }

  /**
   * Get the port number for the preview server
   */
  getPreviewPort(taskId: string): number | undefined {
    const server = this.previewServers.get(taskId)
    return server?.getPort()
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
    return new ClaudeCodeAgent(taskId, agentOptions)
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
   * Cleanup - stop all active preview servers
   */
  async cleanup(): Promise<void> {
    logger.info('LocalEnvironmentDriver: Cleaning up all preview servers', {
      count: this.previewServers.size,
    })

    const tasks = Array.from(this.previewServers.keys())
    await Promise.all(tasks.map((taskId) => this.stopPreview(taskId)))

    this.previewServers.clear()
  }
}
