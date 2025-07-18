import { z } from 'zod'

// Claude settings schema
export const ClaudeSettingsSchema = z.object({
  binDir: z.string().optional(),
  configDir: z.string().optional(),
  oauthToken: z.string().optional(),
})

// Update request schema
export const updateClaudeSettingsSchema = z
  .object({
    binDir: z.string().nullable().optional(),
    configDir: z.string().nullable().optional(),
    oauthToken: z.string().nullable().optional(),
  })
  .strict()

// Response schema - just return the values directly
export const ClaudeSettingsResponseSchema = z.object({
  binDir: z.string(),
  configDir: z.string(),
  oauthToken: z.string(),
})

// Derived types
export type ClaudeSettings = z.infer<typeof ClaudeSettingsSchema>
export type UpdateClaudeSettingsRequest = z.infer<typeof updateClaudeSettingsSchema>
export type ClaudeSettingsResponse = z.infer<typeof ClaudeSettingsResponseSchema>
