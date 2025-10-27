import crypto from 'node:crypto'
import type { Event } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { LocalEnvironmentDriver } from '../../workspace/drivers/local-environment-driver'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type { AgentRunOptions } from '../implementations/base-agent'

/**
 * AgentController - Manages agent lifecycle
 *
 * Responsibilities:
 * - Orchestrate agent start/stop
 * - Update runtime state via RuntimeManager
 * - Delegate actual execution to environment drivers
 *
 * Key principle: Manager orchestrates, RuntimeManager owns state, driver executes operations
 * Driver owns agent instances and process lifecycle (agent map lives in driver)
 */
@injectable()
export class AgentController {
  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(LocalEnvironmentDriver) private readonly driver: LocalEnvironmentDriver,
  ) {}

  /**
   * Start agent execution
   */
  async *startAgent(
    taskId: string,
    agentType: string,
    message: string,
    runOptions?: AgentRunOptions,
  ): AsyncGenerator<Event> {
    logger.info('AgentController: Starting agent', { taskId, agentType })

    // 1. Ensure workspace and get enriched handle with runtime state
    const handle = await this.runtimeManager.ensureWorkspace(taskId)

    // 2. Check if agent is already running
    if (handle.agent.state === 'running') {
      throw new Error(`Agent already running for task: ${taskId}`)
    }

    // 3. Claim agent slot
    const sessionId = crypto.randomUUID()
    await this.runtimeManager.updateAgentState(taskId, {
      state: 'running',
      agentType,
      sessionId,
      startedAt: new Date(),
    })

    try {
      // 4. Get repository config and workspace path
      const repoConfig = await this.repoService.getRepositoryById(handle.repositoryId)
      if (!repoConfig) {
        throw new Error(`Repository not found: ${handle.repositoryId}`)
      }

      const workspacePath = handle.workspacePath
      const workspaceDir = runOptions?.workspaceDir || workspacePath

      // 5. Create agent via driver (driver stores in its map)
      const agentOptions = runOptions?.model ? { model: runOptions.model } : undefined
      const agent = this.driver.createAgent(taskId, agentType, agentOptions)

      // 6. Run agent and stream events
      const agentStream = agent.run(message, {
        ...runOptions,
        workspaceDir,
      })

      for await (const event of agentStream) {
        yield event
      }

      // 7. Agent completed successfully
      logger.info('AgentController: Agent completed', { taskId, agentType, sessionId })
    } catch (error) {
      logger.error('AgentController: Agent failed', error as Error, {
        taskId,
        agentType,
        sessionId,
      })
      throw error
    } finally {
      // 8. Release agent slot (driver handles agent cleanup)
      await this.driver.stopAgent(taskId)
      await this.runtimeManager.updateAgentState(taskId, {
        state: 'idle',
      })
    }
  }

  /**
   * Stop agent execution
   */
  async stopAgent(taskId: string): Promise<void> {
    logger.info('AgentController: Stopping agent', { taskId })

    try {
      // Delegate to driver (driver manages agent map)
      await this.driver.stopAgent(taskId)

      // Update state
      await this.runtimeManager.updateAgentState(taskId, {
        state: 'idle',
      })

      logger.info('AgentController: Agent stopped', { taskId })
    } catch (error) {
      logger.error('AgentController: Failed to stop agent', error as Error, { taskId })
      throw error
    }
  }

  /**
   * Get agent status
   */
  async getStatus(taskId: string): Promise<{
    agentType: string | undefined
    state: 'idle' | 'running' | 'none'
    sessionId: string | undefined
  }> {
    const snapshot = await this.runtimeManager.getSnapshot(taskId)
    if (!snapshot?.agent) {
      return {
        agentType: undefined,
        state: 'none',
        sessionId: undefined,
      }
    }

    return {
      agentType: snapshot.agent.agentType,
      state: snapshot.agent.state,
      sessionId: snapshot.agent.sessionId,
    }
  }

  /**
   * Cleanup - stop all active agents via driver
   */
  async cleanup(): Promise<void> {
    logger.info('AgentController: Delegating cleanup to driver')

    // Driver handles cleanup of all agents
    await this.driver.cleanup()
  }
}
