/**
 * Legacy Slack channel mapping types.
 * Used by the old Node.js channel mapping API (/api/v1/channels/slack/mappings).
 */

export interface SlackChannelMappingResponse {
  id: number
  channelId: string
  teamId: string
  repositoryId: string
  repositoryName: string | null
  createdBy: string
  createdAt: string
  updatedAt: string
}

export interface CreateSlackChannelMappingRequest {
  channelId: string
  teamId: string
  repositoryId: string
}

export interface UpdateSlackChannelMappingRequest {
  repositoryId: string
}
