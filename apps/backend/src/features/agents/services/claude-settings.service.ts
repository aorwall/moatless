import type {
  ClaudeSettings,
  ClaudeSettingsResponse,
  UpdateClaudeSettingsRequest,
} from '@moatless/types'
import { injectable } from 'inversify'
import type { SettingsService } from '../../../services/settings.service'

@injectable()
export class ClaudeSettingsService {
  constructor(private settingsService: SettingsService) {}

  /**
   * Get Claude settings with current values or defaults
   */
  getClaudeSettings(): Required<ClaudeSettings> {
    return this.settingsService.getClaudeSettings()
  }

  /**
   * Get Claude settings response with validation status
   */
  async getClaudeSettingsResponse(): Promise<ClaudeSettingsResponse> {
    return await this.settingsService.getClaudeSettingsResponse()
  }

  /**
   * Update Claude settings section
   */
  async updateClaudeSettingsSection(
    update: UpdateClaudeSettingsRequest,
  ): Promise<ClaudeSettingsResponse> {
    return await this.settingsService.updateClaudeSettingsSection(update)
  }
}
