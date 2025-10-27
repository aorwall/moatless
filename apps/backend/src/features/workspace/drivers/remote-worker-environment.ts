import type { ServerStatus, ShellCommandLogEvent } from '@moatless/types'
import { injectable } from 'inversify'
import type { BaseAgent } from '../../../agents/base-agent'
import { logger } from '../../../utils/logger'
import type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  PreviewConfiguration,
  RunCommandOptions,
} from './base-environment-driver'

/**
 * RemoteWorkerEnvironment - RPC client for worker-backed execution
 *
 * This driver delegates all operations to a remote worker via RPC.
 * It's a thin client that marshals requests and unmarshals responses.
 *
 * NOTE: This is a stub implementation. Full implementation will be added
 * when we implement worker support in a future phase.
 */
@injectable()
export class RemoteWorkerEnvironment implements BaseEnvironmentDriver {
  constructor() {
    logger.info('RemoteWorkerEnvironment: Initialized (stub implementation)')
  }

  // biome-ignore lint/suspicious/useAwait: Generator function needs async for yield
  async *runCommand(
    workspacePath: string,
    command: string,
    _options?: RunCommandOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    logger.warn('RemoteWorkerEnvironment: runCommand not yet implemented', {
      workspacePath,
      command,
    })
    // Yield to satisfy generator requirement
    yield {
      type: 'system',
      timestamp: new Date().toISOString(),
      text: 'Error: Worker support not yet implemented\n',
    }
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async startPreview(config: PreviewConfiguration): Promise<ServerStatus> {
    logger.warn('RemoteWorkerEnvironment: startPreview not yet implemented', {
      taskId: config.taskId,
    })
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async stopPreview(taskId: string): Promise<void> {
    logger.warn('RemoteWorkerEnvironment: stopPreview not yet implemented', { taskId })
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async getPreviewStatus(taskId: string): Promise<ServerStatus> {
    logger.warn('RemoteWorkerEnvironment: getPreviewStatus not yet implemented', { taskId })
    return {
      status: 'stopped',
      uptimeSeconds: 0,
    }
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async getPreviewLogs(taskId: string, lines?: number): Promise<string> {
    logger.warn('RemoteWorkerEnvironment: getPreviewLogs not yet implemented', { taskId, lines })
    return 'Worker support not yet implemented'
  }

  // biome-ignore lint/suspicious/useAwait: Generator function needs async for yield
  async *streamPreviewLogs(taskId: string): AsyncGenerator<{
    type: string
    data: { text?: string; error?: string; timestamp: string }
  }> {
    logger.warn('RemoteWorkerEnvironment: streamPreviewLogs not yet implemented', { taskId })
    yield {
      type: 'error',
      data: {
        error: 'Worker support not yet implemented',
        timestamp: new Date().toISOString(),
      },
    }
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async proxyPreviewRequest(
    taskId: string,
    _request: {
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
    logger.warn('RemoteWorkerEnvironment: proxyPreviewRequest not yet implemented', { taskId })
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  getPreviewPort(taskId: string): number | undefined {
    logger.warn('RemoteWorkerEnvironment: getPreviewPort not yet implemented', { taskId })
    return undefined
  }

  createAgent(taskId: string, agentType: string, _options?: AgentCreationOptions): BaseAgent {
    logger.warn('RemoteWorkerEnvironment: createAgent not yet implemented', {
      taskId,
      agentType,
    })
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async readFile(workspacePath: string, filePath: string): Promise<string> {
    logger.warn('RemoteWorkerEnvironment: readFile not yet implemented', {
      workspacePath,
      filePath,
    })
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async executeSetupCommands(
    workspacePath: string,
    commands: string[],
    _envVars?: Record<string, string>,
  ): Promise<void> {
    logger.warn('RemoteWorkerEnvironment: executeSetupCommands not yet implemented', {
      workspacePath,
      commands,
    })
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }
}
