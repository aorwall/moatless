import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger.js'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service.js'
import type { PreviewState } from '../../workspace/types/workspace-runtime.types.js'
import type { PreviewStateBridge } from './preview-state-bridge.interface.js'

/**
 * PreviewStateBridgeService - Allows preview feature to update workspace preview state
 *
 * This service is implemented in the preview feature and injected into workspace feature
 * to break circular dependencies while allowing preview state updates.
 */
@injectable()
export class PreviewStateBridgeService implements PreviewStateBridge {
  private readonly serviceLogger: typeof logger

  constructor(
    @inject(WorkspaceRuntimeManager)
    private readonly runtimeManager: WorkspaceRuntimeManager,
  ) {
    this.serviceLogger = logger.child({ service: 'preview-state-bridge' })
  }

  /**
   * Update preview state in workspace runtime snapshot
   */
  async updatePreviewState(taskId: string, updates: Partial<PreviewState>): Promise<void> {
    this.serviceLogger.debug('Updating preview state', { taskId, updates })
    await this.runtimeManager.updatePreviewState(taskId, updates)
  }
}
