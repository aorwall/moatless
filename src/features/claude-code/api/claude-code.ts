import { apiClient } from '@/lib/api/client';
import type {
  ClaudeCodeSettingsResponse,
  ClaudeCodeToolsResponse,
  UpdateClaudeCodeSettingsRequest,
} from '@/types/claude-code';

export const claudeCodeApi = {
  async getSettings(): Promise<ClaudeCodeSettingsResponse> {
    return await apiClient.get<ClaudeCodeSettingsResponse>(
      '/api/v1/claude-code/settings',
    );
  },

  async updateSettings(
    request: UpdateClaudeCodeSettingsRequest,
  ): Promise<ClaudeCodeSettingsResponse> {
    return await apiClient.put<ClaudeCodeSettingsResponse>(
      '/api/v1/claude-code/settings',
      request,
    );
  },

  async deleteSettings(): Promise<void> {
    await apiClient.delete('/api/v1/claude-code/settings');
  },

  async getAvailableTools(): Promise<ClaudeCodeToolsResponse> {
    return await apiClient.get<ClaudeCodeToolsResponse>(
      '/api/v1/claude-code/tools',
    );
  },
};
