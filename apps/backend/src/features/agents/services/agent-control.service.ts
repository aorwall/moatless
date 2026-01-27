import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { agentExecutionTotal } from '../../metrics'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'

/**
 * AgentControlService - Controls agent execution state
 *
 * Responsibilities:
 * - Stop running agents (sends signal to sandbox, state updates via events)
 * - Get agent status
 *
 * State updates are handled by TaskEventHandler when sandbox emits events.
 * Request building is handled by AgentRequestBuilder.
 */
@injectable()
export class AgentControlService {
  constructor(
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
  ) {}

  /**
   * Stop agent execution
   *
   * This method sends the stop signal to the sandbox worker.
   * State updates (agent state to 'idle', iteration to 'completed') are handled
   * by TaskEventHandler when the sandbox emits the 'agent.stopped' event.
   */
  async stopAgent(taskId: string): Promise<void> {
    logger.info({ taskId }, 'AgentControlService: Stopping agent')

    try {
      // Stop agent via RuntimeManager - sends stop signal to sandbox worker
      // The sandbox will emit 'agent.stopped' event which triggers state updates
      await this.runtimeManager.stopAgentAndInvalidate(taskId)

      // Track stopped metric (agent_type unknown since we don't have context)
      agentExecutionTotal.labels({ status: 'stopped', agent_type: 'unknown' }).inc()

      logger.info({ taskId }, 'AgentControlService: Stop signal sent to sandbox')
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
