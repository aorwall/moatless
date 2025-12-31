import type { GitChanges, GitStatus, ShellCommandLogEvent } from '@moatless/types'
import { injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { BaseAgent } from '../../agents/agent/base-agent'
import type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  FileSearchRequest,
  FileSearchResult,
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
  readonly taskId: string = 'stub'

  constructor() {
    logger.info('RemoteWorkerEnvironment: Initialized (stub implementation)')
  }

  // biome-ignore lint/suspicious/useAwait: Generator function needs async for yield
  async *runCommand(
    command: string,
    _options?: RunCommandOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    logger.warn(
      {
        command,
      },
      'RemoteWorkerEnvironment: runCommand not yet implemented',
    )
    // Yield to satisfy generator requirement
    yield {
      type: 'system',
      timestamp: new Date().toISOString(),
      text: 'Error: Worker support not yet implemented\n',
    }
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  createAgent(agentType: string, _options?: AgentCreationOptions): BaseAgent {
    logger.warn(
      {
        agentType,
      },
      'RemoteWorkerEnvironment: createAgent not yet implemented',
    )
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async stopAgent(): Promise<void> {
    logger.warn('RemoteWorkerEnvironment: stopAgent not yet implemented')
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async readFile(filePath: string): Promise<string> {
    logger.warn(
      {
        filePath,
      },
      'RemoteWorkerEnvironment: readFile not yet implemented',
    )
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async writeFile(filePath: string, _content: string): Promise<void> {
    logger.warn(
      {
        filePath,
      },
      'RemoteWorkerEnvironment: writeFile not yet implemented',
    )
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async listFiles(relativePath?: string, _recursive?: boolean): Promise<string[]> {
    logger.warn(
      {
        relativePath,
      },
      'RemoteWorkerEnvironment: listFiles not yet implemented',
    )
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async searchFiles(request: FileSearchRequest): Promise<FileSearchResult> {
    logger.warn(
      {
        query: request.query,
      },
      'RemoteWorkerEnvironment: searchFiles not yet implemented',
    )
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async getGitStatus(): Promise<GitStatus> {
    logger.warn('RemoteWorkerEnvironment: getGitStatus not yet implemented')
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }

  // biome-ignore lint/suspicious/useAwait: Stub implementation, will use await in full version
  async getGitChanges(): Promise<GitChanges> {
    logger.warn('RemoteWorkerEnvironment: getGitChanges not yet implemented')
    throw new Error('RemoteWorkerEnvironment: Worker support not yet implemented')
  }
}
