import { z } from 'zod'

// General settings schema
export const GeneralSettingsSchema = z.object({
  dataDir: z.string().optional(),
  repositoriesDir: z.string().optional(),
  defaultWorkspaceType: z.enum(['docker', 'local']).optional(),
})

// Update request schema
export const updateGeneralSettingsSchema = z
  .object({
    dataDir: z.string().nullable().optional(),
    repositoriesDir: z.string().nullable().optional(),
    defaultWorkspaceType: z.enum(['docker', 'local']).nullable().optional(),
  })
  .strict()

// Response schema
export const GeneralSettingsResponseSchema = z.object({
  settings: GeneralSettingsSchema,
  defaults: z.object({
    dataDir: z.string(),
    repositoriesDir: z.string(),
    defaultWorkspaceType: z.enum(['docker', 'local']),
  }),
})

// Derived types
export type UpdateGeneralSettingsRequest = z.infer<typeof updateGeneralSettingsSchema>
export type GeneralSettingsResponse = z.infer<typeof GeneralSettingsResponseSchema>
