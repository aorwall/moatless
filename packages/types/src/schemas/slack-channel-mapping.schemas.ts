import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'

/**
 * Schema for Slack channel mapping.
 */
export const SlackChannelMappingSchema = z.object({
  id: z.number().optional(),
  channelId: z.string().min(1, 'Channel ID is required'),
  teamId: z.string().min(1, 'Team ID is required'),
  repositoryId: z.string().min(1, 'Repository ID is required'),
  createdBy: z.string().min(1, 'Creator user ID is required'),
  createdAt: dateSchema,
  updatedAt: dateSchema,
})

/**
 * Schema for creating a Slack channel mapping.
 */
export const CreateSlackChannelMappingSchema = z.object({
  channelId: z.string().min(1, 'Channel ID is required'),
  teamId: z.string().min(1, 'Team ID is required'),
  repositoryId: z.string().min(1, 'Repository ID is required'),
})

/**
 * Schema for updating a Slack channel mapping.
 */
export const UpdateSlackChannelMappingSchema = z.object({
  repositoryId: z.string().min(1, 'Repository ID is required'),
})

/**
 * Schema for Slack channel mapping response with repository details.
 */
export const SlackChannelMappingResponseSchema = z.object({
  id: z.number(),
  channelId: z.string(),
  teamId: z.string(),
  repositoryId: z.string(),
  repositoryName: z.string().nullable(),
  createdBy: z.string(),
  createdAt: z.string(),
  updatedAt: z.string(),
})

/**
 * Schema for list of Slack channel mappings response.
 */
export const SlackChannelMappingListResponseSchema = z.array(SlackChannelMappingResponseSchema)

// Type exports
export type SlackChannelMapping = z.infer<typeof SlackChannelMappingSchema>
export type CreateSlackChannelMappingRequest = z.infer<typeof CreateSlackChannelMappingSchema>
export type UpdateSlackChannelMappingRequest = z.infer<typeof UpdateSlackChannelMappingSchema>
export type SlackChannelMappingResponse = z.infer<typeof SlackChannelMappingResponseSchema>
export type SlackChannelMappingListResponse = z.infer<typeof SlackChannelMappingListResponseSchema>
