import { randomUUID } from 'node:crypto'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import type {
  PreviewServerLifecycleSink,
  PreviewServerSnapshot,
  PreviewServerUpdate,
  ServerStatusChangeInfo,
} from '../types/index.js'

/**
 * PreviewRuntimeService - Manages preview server events (Kubernetes mode)
 *
 * In Kubernetes mode, preview servers run in sandbox pods. This service:
 * - Implements PreviewServerLifecycleSink for state updates from sandbox
 * - Emits SSE events for UI updates
 *
 * Preview state is NOT persisted — sandbox tracks it internally.
 * This service only emits events for real-time frontend updates via SSE.
 *
 * Note: start/stop/restart operations are handled by the environment driver,
 * which delegates to the sandbox pod. This service only handles events.
 */
@injectable()
export class PreviewRuntimeService implements PreviewServerLifecycleSink {
  private readonly serviceLogger: typeof logger

  constructor(@inject(EventService) private readonly eventService: EventService) {
    this.serviceLogger = logger.child({ service: 'preview-runtime' })
  }

  /**
   * PreviewServerLifecycleSink implementation - register server snapshot
   * Preview state is ephemeral — only log for debugging.
   */
  register(serverId: string, snapshot: PreviewServerSnapshot): Promise<void> {
    this.serviceLogger.debug(
      {
        serverId,
        status: snapshot.status,
      },
      'Registering preview server state (ephemeral)',
    )
    return Promise.resolve()
  }

  /**
   * PreviewServerLifecycleSink implementation - update server state
   * Preview state is ephemeral — only log for debugging.
   */
  update(serverId: string, changes: PreviewServerUpdate): Promise<void> {
    this.serviceLogger.debug({ serverId, changes }, 'Updating preview server state (ephemeral)')
    return Promise.resolve()
  }

  /**
   * PreviewServerLifecycleSink implementation - remove server state
   * Preview state is ephemeral — only log for debugging.
   */
  remove(serverId: string): Promise<void> {
    this.serviceLogger.debug({ serverId }, 'Removing preview server state (ephemeral)')
    return Promise.resolve()
  }

  /**
   * PreviewServerLifecycleSink implementation - handle status changes
   * Emits ServerEvent to EventService for SSE streaming
   */
  async onStatusChange?(serverId: string, statusInfo: ServerStatusChangeInfo): Promise<void> {
    this.serviceLogger.debug(
      {
        serverId,
        status: statusInfo.status,
      },
      'Preview server status changed, emitting event',
    )

    try {
      // Map status to event subType
      const subType = statusInfo.status as 'starting' | 'started' | 'stopped' | 'failed'

      // Emit ServerEvent
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: 'server',
        subType,
        taskId: serverId,
        timestamp: new Date(),
        isError: statusInfo.status === 'failed',
        payload: {
          serverName: 'preview',
          status: statusInfo.status,
          error: statusInfo.error,
        },
      })

      this.serviceLogger.debug(
        {
          serverId,
          status: statusInfo.status,
        },
        'ServerEvent emitted successfully',
      )
    } catch (error) {
      // Log error but don't throw - event emission should not break server lifecycle
      this.serviceLogger.error(
        {
          err: error as Error,
          serverId,
          status: statusInfo.status,
        },
        'Failed to emit ServerEvent',
      )
    }
  }

  /**
   * PreviewServerLifecycleSink implementation - handle Playwright actions
   * Emits PreviewEvent with playwright-action subType for SSE streaming
   */
  async onPlaywrightAction?(
    serverId: string,
    actionInfo: import('../types/index.js').PlaywrightActionInfo,
  ): Promise<void> {
    this.serviceLogger.debug(
      {
        serverId,
        type: actionInfo.type,
        url: actionInfo.url,
        mode: actionInfo.mode,
      },
      'Playwright action occurred, emitting event',
    )

    try {
      // Emit PreviewEvent for Playwright action
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: 'preview',
        subType: 'playwright-action',
        taskId: serverId,
        timestamp: new Date(),
        isError: false,
        payload: {
          actionType: actionInfo.type as
            | 'navigate'
            | 'click'
            | 'evaluate'
            | 'fillForm'
            | 'snapshot',
          url: actionInfo.url,
          mode: actionInfo.mode,
          timestamp: actionInfo.timestamp,
        },
      })

      this.serviceLogger.debug(
        {
          serverId,
          actionType: actionInfo.type,
        },
        'PlaywrightAction event emitted successfully',
      )
    } catch (error) {
      // Log error but don't throw - event emission should not break Playwright actions
      this.serviceLogger.error(
        {
          err: error as Error,
          serverId,
          actionType: actionInfo.type,
        },
        'Failed to emit PlaywrightAction event',
      )
    }
  }

  /**
   * PreviewServerLifecycleSink implementation - handle Playwright mode changes
   * Emits PreviewEvent with mode-change subType for SSE streaming
   */
  async onPlaywrightModeChange?(serverId: string, mode: 'idle' | 'user' | 'agent'): Promise<void> {
    this.serviceLogger.debug(
      {
        serverId,
        mode,
      },
      'Playwright mode changed, emitting event',
    )

    try {
      // Emit PreviewEvent for mode change
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: 'preview',
        subType: 'mode-change',
        taskId: serverId,
        timestamp: new Date(),
        isError: false,
        payload: {
          mode,
        },
      })

      this.serviceLogger.debug(
        {
          serverId,
          mode,
        },
        'PlaywrightModeChange event emitted successfully',
      )
    } catch (error) {
      // Log error but don't throw - event emission should not break mode changes
      this.serviceLogger.error(
        {
          err: error as Error,
          serverId,
          mode,
        },
        'Failed to emit PlaywrightModeChange event',
      )
    }
  }
}
