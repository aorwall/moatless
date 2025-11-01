import { logger } from '../../../utils/logger.js'
import type { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service.js'
import { getPreviewInspectorScript } from '../inspector/inspector-script.js'
import type { TargetInfo, TargetResolver } from '../types/index.js'

/**
 * RegistryTargetResolver - Resolves preview server targets from workspace snapshots
 *
 * Used by PreviewProxyService to route requests to the correct preview server port.
 * Queries workspace runtime snapshots instead of a separate preview_servers table.
 */
export class RegistryTargetResolver implements TargetResolver {
  private readonly serviceLogger: typeof logger

  constructor(private readonly runtimeManager: WorkspaceRuntimeManager) {
    this.serviceLogger = logger.child({ service: 'registry-target-resolver' })
  }

  async resolve(serverId: string): Promise<TargetInfo | null> {
    try {
      const snapshot = await this.runtimeManager.getSnapshot(serverId)
      if (!snapshot?.preview) {
        return null
      }

      const { status, port, inspectorEnabled } = snapshot.preview

      if (status !== 'started' || !port) {
        return null
      }

      const scriptContent =
        inspectorEnabled === false ? undefined : getPreviewInspectorScript(serverId)

      return {
        port,
        scriptContent,
      }
    } catch (error) {
      this.serviceLogger.warn('RegistryTargetResolver failed to resolve target', {
        serverId,
        error: error instanceof Error ? error.message : String(error),
      })
      return null
    }
  }
}
