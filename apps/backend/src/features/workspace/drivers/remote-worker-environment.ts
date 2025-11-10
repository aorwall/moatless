import type { ShellCommandLogEvent } from '@moatless/types'
import { injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { BaseAgent } from '../../agents/agent/base-agent'
import type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
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

  createAgent(taskId: string, agentType: string, _options?: AgentCreationOptions): BaseAgent {
    logger.warn('RemoteWorkerEnvironment: createAgent not yet implemented', {
      taskId,
      agentType,
    })
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async stopAgent(taskId: string): Promise<void> {
    logger.warn('RemoteWorkerEnvironment: stopAgent not yet implemented', { taskId })
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
}
