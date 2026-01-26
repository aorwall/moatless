import { injectable } from 'inversify'
import { logger } from '../../../utils/logger'

/**
 * Information about a pending interactive tool request.
 */
export interface PendingToolInfo {
  toolName: 'AskUserQuestion' | 'ExitPlanMode'
  toolInput: Record<string, unknown>
  timestamp: Date
}

/**
 * PendingResponseService - Tracks pending interactive tool requests by taskId.
 *
 * When an agent uses AskUserQuestion or ExitPlanMode, it emits a pending_response event.
 * This service tracks those pending requests so that when a user responds via Slack/Email,
 * we know to route the response to the user-input-response endpoint instead of sendMessage.
 *
 * State lifecycle:
 * - Set when agent:pending_response event is received
 * - Cleared when:
 *   - User responds (via UI, Slack, or Email)
 *   - Agent iteration completes or errors
 *   - Task is closed
 */
@injectable()
export class PendingResponseService {
  private pendingByTaskId = new Map<string, PendingToolInfo>()

  /**
   * Set a pending interactive tool request for a task.
   * Called when agent:pending_response event is received.
   */
  setPending(
    taskId: string,
    toolName: 'AskUserQuestion' | 'ExitPlanMode',
    toolInput: Record<string, unknown>,
  ): void {
    const info: PendingToolInfo = {
      toolName,
      toolInput,
      timestamp: new Date(),
    }
    this.pendingByTaskId.set(taskId, info)
    logger.debug({ taskId, toolName }, 'Pending interactive tool request set')
  }

  /**
   * Get the pending interactive tool info for a task, if any.
   */
  getPending(taskId: string): PendingToolInfo | undefined {
    return this.pendingByTaskId.get(taskId)
  }

  /**
   * Check if a task has a pending interactive tool request.
   */
  hasPending(taskId: string): boolean {
    return this.pendingByTaskId.has(taskId)
  }

  /**
   * Clear the pending interactive tool request for a task.
   * Called when user responds or iteration completes/errors.
   */
  clearPending(taskId: string): void {
    if (this.pendingByTaskId.has(taskId)) {
      this.pendingByTaskId.delete(taskId)
      logger.debug({ taskId }, 'Pending interactive tool request cleared')
    }
  }

  /**
   * Get all pending task IDs (for debugging/monitoring).
   */
  getAllPendingTaskIds(): string[] {
    return Array.from(this.pendingByTaskId.keys())
  }
}
