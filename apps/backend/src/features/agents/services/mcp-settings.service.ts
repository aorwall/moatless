import type { McpSettings, McpSettingsResponse, UpdateMcpSettingsRequest } from '@moatless/types'
import { injectable } from 'inversify'
import type { SettingsService } from '../../../services/settings.service'

@injectable()
export class McpSettingsService {
  constructor(private settingsService: SettingsService) {}

  /**
   * Get MCP settings
   */
  async getMcpSettings(): Promise<Required<McpSettings>> {
    return await this.settingsService.getMcpSettings()
  }

  /**
   * Get MCP settings response with defaults
   */
  getMcpSettingsResponse(): McpSettingsResponse {
    return this.settingsService.getMcpSettingsResponse()
  }

  /**
   * Update MCP settings section
   */
  async updateMcpSettingsSection(update: UpdateMcpSettingsRequest): Promise<McpSettingsResponse> {
    return await this.settingsService.updateMcpSettingsSection(update)
  }
}
