import { apiClient } from '@/lib/api/client'
import type {
  CreateSlackChannelMappingRequest,
  SlackChannelMappingResponse,
  UpdateSlackChannelMappingRequest,
} from '@/lib/types/slack-channel-mappings'

const BASE_PATH = '/api/v1/channels/slack/mappings'

export const slackChannelMappingsApi = {
  /**
   * List all Slack channel mappings.
   */
  async listMappings(): Promise<SlackChannelMappingResponse[]> {
    return await apiClient.get<SlackChannelMappingResponse[]>(BASE_PATH)
  },

  /**
   * Get Slack channel mappings for a specific repository.
   */
  async getMappingsForRepository(repositoryId: string): Promise<SlackChannelMappingResponse[]> {
    const allMappings = await apiClient.get<SlackChannelMappingResponse[]>(BASE_PATH)
    return allMappings.filter((mapping) => mapping.repositoryId === repositoryId)
  },

  /**
   * Get a specific channel mapping.
   */
  async getMapping(channelId: string, teamId: string): Promise<SlackChannelMappingResponse> {
    return await apiClient.get<SlackChannelMappingResponse>(
      `${BASE_PATH}/${encodeURIComponent(channelId)}/${encodeURIComponent(teamId)}`,
    )
  },

  /**
   * Create a new channel mapping.
   */
  async createMapping(
    request: CreateSlackChannelMappingRequest,
  ): Promise<SlackChannelMappingResponse> {
    return await apiClient.post<SlackChannelMappingResponse>(BASE_PATH, request)
  },

  /**
   * Update a channel mapping.
   */
  async updateMapping(
    channelId: string,
    teamId: string,
    request: UpdateSlackChannelMappingRequest,
  ): Promise<SlackChannelMappingResponse> {
    return await apiClient.put<SlackChannelMappingResponse>(
      `${BASE_PATH}/${encodeURIComponent(channelId)}/${encodeURIComponent(teamId)}`,
      request,
    )
  },

  /**
   * Delete a channel mapping.
   */
  async deleteMapping(channelId: string, teamId: string): Promise<void> {
    await apiClient.delete(
      `${BASE_PATH}/${encodeURIComponent(channelId)}/${encodeURIComponent(teamId)}`,
    )
  },
}
