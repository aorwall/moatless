import { randomUUID } from 'node:crypto'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type { PreviewState } from '../../workspace/types/workspace-runtime.types'
import type {
  PreviewServerLifecycleSink,
  PreviewServerSnapshot,
  PreviewServerUpdate,
  ServerStatusChangeInfo,
} from '../types/index.js'

/**
 * PreviewRuntimeService - Manages preview server state and events (Kubernetes mode)
 *
 * In Kubernetes mode, preview servers run in sandbox pods. This service:
 * - Implements PreviewServerLifecycleSink for state updates from sandbox
 * - Emits SSE events for UI updates
 * - Tracks preview state in workspace snapshots
 *
 * Note: start/stop/restart operations are handled by the environment driver,
 * which delegates to the sandbox pod. This service only handles state/events.
 */
@injectable()
export class PreviewRuntimeService implements PreviewServerLifecycleSink {
  private readonly serviceLogger: typeof logger

  constructor(
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(EventService) private readonly eventService: EventService,
  ) {
    this.serviceLogger = logger.child({ service: 'preview-runtime' })
  }

  /**
   * PreviewServerLifecycleSink implementation - register server snapshot
   */
  async register(serverId: string, snapshot: PreviewServerSnapshot): Promise<void> {
    this.serviceLogger.debug(
      {
        serverId,
        status: snapshot.status,
      },
      'Registering preview server state',
    )
    await this.runtimeManager.updatePreviewState(serverId, {
      status: snapshot.status,
      port: snapshot.port,
      endpoint: snapshot.internalUrl,
      proxyUrl: snapshot.externalUrl,
      pid: snapshot.pid,
      inspectorEnabled: snapshot.inspectorEnabled,
      lastError: snapshot.error,
    })
  }

  /**
   * PreviewServerLifecycleSink implementation - update server state
   */
  async update(serverId: string, changes: PreviewServerUpdate): Promise<void> {
    this.serviceLogger.debug({ serverId, changes }, 'Updating preview server state')

    // Build update object with only defined properties (exactOptionalPropertyTypes)
    const updates: Partial<PreviewState> = {}
    if (changes.status !== undefined) {
      updates.status = changes.status
      // Set lastStartedAt when status changes to 'started'
      if (changes.status === 'started') {
        updates.lastStartedAt = new Date()
      }
    }
    if (changes.port !== undefined) {
      updates.port = changes.port
    }
    if (changes.internalUrl !== undefined) {
      updates.endpoint = changes.internalUrl
    }
    if (changes.externalUrl !== undefined) {
      updates.proxyUrl = changes.externalUrl
    }
    if (changes.pid !== undefined) {
      updates.pid = changes.pid
    }
    if (changes.inspectorEnabled !== undefined) {
      updates.inspectorEnabled = changes.inspectorEnabled
    }
    if (changes.error !== undefined) {
      updates.lastError = changes.error
    }

    await this.runtimeManager.updatePreviewState(serverId, updates)
  }

  /**
   * PreviewServerLifecycleSink implementation - remove server state
   */
  async remove(serverId: string): Promise<void> {
    this.serviceLogger.debug({ serverId }, 'Removing preview server state')
    // Mark as stopped when removed
    await this.runtimeManager.updatePreviewState(serverId, {
      status: 'stopped',
      port: undefined,
      endpoint: undefined,
      proxyUrl: undefined,
      pid: undefined,
      lastStoppedAt: new Date(),
    })
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
          status: statusInfo.status,
          port: statusInfo.port,
          pid: statusInfo.pid,
          url: statusInfo.externalUrl, // Frontend gets external URL (proxy URL)
          error: statusInfo.error,
          uptimeSeconds: statusInfo.uptimeSeconds,
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

  /**
   * Mark a server's workspace as dirty (code changes detected)
   * Updates workspace state directly via RuntimeManager
   */
  async markServerDirty(taskId: string): Promise<void> {
    await this.runtimeManager.updatePreviewState(taskId, {
      dirtyContext: true,
    })
  }

  /**
   * Clear a server's dirty context flag
   * Updates workspace state directly via RuntimeManager
   */
  async clearServerDirty(taskId: string): Promise<void> {
    await this.runtimeManager.updatePreviewState(taskId, {
      dirtyContext: false,
    })
  }

  /**
   * Check if a server's workspace context is dirty
   */
  async isServerDirty(taskId: string): Promise<boolean> {
    try {
      const snapshot = await this.runtimeManager.getSnapshot(taskId)
      return snapshot?.preview?.dirtyContext ?? false
    } catch {
      return false
    }
  }
}
