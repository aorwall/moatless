import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { agentExecutionTotal } from '../../metrics'
import { TaskIterationService } from '../../tasks/services/task-iteration.service'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'

/**
 * AgentControlService - Controls agent execution state
 *
 * Responsibilities:
 * - Stop running agents
 * - Get agent status
 *
 * This service handles control operations that affect agent state.
 * Request building is handled by AgentRequestBuilder.
 */
@injectable()
export class AgentControlService {
  constructor(
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(TaskIterationService) private readonly taskIterationService: TaskIterationService,
  ) {}

  /**
   * Stop agent execution
   */
  async stopAgent(taskId: string): Promise<void> {
    logger.info({ taskId }, 'AgentControlService: Stopping agent')

    try {
      // Stop agent via RuntimeManager
      await this.runtimeManager.stopAgentAndInvalidate(taskId)

      // Update state
      await this.runtimeManager.updateAgentState(taskId, {
        state: 'idle',
      })

      // Mark active iteration as completed with "Stopped by user"
      await this.taskIterationService.markIterationAsCompleted(taskId, 'Stopped by user')

      // Track stopped metric (agent_type unknown since we don't have context)
      agentExecutionTotal.labels({ status: 'stopped', agent_type: 'unknown' }).inc()

      logger.info({ taskId }, 'AgentControlService: Agent stopped')
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'AgentControlService: Failed to stop agent')
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
}
