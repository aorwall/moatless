import { z } from 'zod'

// GitHub settings schema
export const GitHubSettingsSchema = z.object({
  personalAccessToken: z.string().optional(),
  userName: z.string().optional(),
  userEmail: z.string().optional(),
})

// Update request schema
export const updateGitHubSettingsSchema = z
  .object({
    personalAccessToken: z.string().nullable().optional(),
    userName: z.string().nullable().optional(),
    userEmail: z.string().nullable().optional(),
  })
  .strict()

// Response schema
export const GitHubSettingsResponseSchema = z.object({
  settings: GitHubSettingsSchema,
  defaults: z.object({
    personalAccessToken: z.string(),
    userName: z.string(),
    userEmail: z.string(),
  }),
})

// Derived types
export type GitHubSettings = z.infer<typeof GitHubSettingsSchema>
export type UpdateGitHubSettingsRequest = z.infer<typeof updateGitHubSettingsSchema>
export type GitHubSettingsResponse = z.infer<typeof GitHubSettingsResponseSchema>
