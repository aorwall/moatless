import { logger } from '../../../utils/logger.js'
import { getPreviewInspectorScript } from '../inspector/inspector-script.js'
import type { PreviewRegistryService } from '../services/preview-registry.service.js'
import type { TargetInfo, TargetResolver } from '../types/index.js'

export class RegistryTargetResolver implements TargetResolver {
  private readonly serviceLogger: typeof logger

  constructor(private readonly registry: PreviewRegistryService) {
    this.serviceLogger = logger.child({ service: 'registry-target-resolver' })
  }

  async resolve(serverId: string): Promise<TargetInfo | null> {
    try {
      const record = await this.registry.getRecord(serverId)
      if (!record) {
        return null
      }

      if (record.status !== 'started' || !record.port) {
        return null
      }

      const scriptContent =
        record.inspectorEnabled === false ? undefined : getPreviewInspectorScript(serverId)

      return {
        port: record.port,
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
