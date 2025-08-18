import { z } from 'zod'

export const PublishSettingsSchema = z.object({
  netlifyAccessToken: z.string().optional(),
})

export const PublishSettingsResponseSchema = z.object({
  settings: z.object({
    netlifyAccessToken: z.string().optional(),
  }),
  defaults: z.object({
    netlifyAccessToken: z.string(),
  }),
})

export const UpdatePublishSettingsRequestSchema = z.object({
  netlifyAccessToken: z.string().nullable().optional(),
})

// Type exports
export type PublishSettings = z.infer<typeof PublishSettingsSchema>
export type PublishSettingsResponse = z.infer<typeof PublishSettingsResponseSchema>
export type UpdatePublishSettingsRequest = z.infer<typeof UpdatePublishSettingsRequestSchema>
