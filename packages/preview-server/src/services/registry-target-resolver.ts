import { getPreviewInspectorScript } from '../core/inspector-script.js'
import type { Logger, TargetInfo, TargetResolver } from '../types/index.js'
import type { PreviewServerRegistry } from './preview-server-registry.js'

export class RegistryTargetResolver implements TargetResolver {
  constructor(
    private readonly registry: PreviewServerRegistry,
    private readonly logger?: Logger,
  ) {}

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
      this.logger?.warn('RegistryTargetResolver failed to resolve target', {
        serverId,
        error: error instanceof Error ? error.message : String(error),
      })
      return null
    }
  }
}
