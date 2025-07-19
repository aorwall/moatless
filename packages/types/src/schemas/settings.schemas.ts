import { z } from 'zod'
import { ClaudeSettingsSchema } from './settings/claude.schemas.js'
import { GitHubSettingsSchema } from './settings/github.schemas.js'

export const ApplicationSettingsSchema = z.object({
  dataDir: z.string().optional(),
  defaultWorkspaceType: z.enum(['docker', 'local']).optional(),
  // Claude settings at root level
  binDir: z.string().optional(),
  configDir: z.string().optional(),
  oauthToken: z.string().optional(),
  // Nested settings
  claude: ClaudeSettingsSchema.optional(),
  github: GitHubSettingsSchema.optional(),
})

export const SystemStatusCheckSchema = z.object({
  claude: z.object({
    binPath: z.string().nullable(),
    configPath: z.string().nullable(),
    isValidBin: z.boolean(),
    isValidConfig: z.boolean(),
  }),
  docker: z.object({
    isRunning: z.boolean(),
    version: z.string().nullable(),
  }),
})

// Update settings schema
export const updateSettingsSchema = z
  .object({
    dataDir: z.string().nullable().optional(),
    defaultWorkspaceType: z.enum(['docker', 'local']).nullable().optional(),
    binDir: z.string().nullable().optional(),
    configDir: z.string().nullable().optional(),
    oauthToken: z.string().nullable().optional(),
    claude: z
      .object({
        binDir: z.string().nullable().optional(),
        configDir: z.string().nullable().optional(),
        oauthToken: z.string().nullable().optional(),
      })
      .nullable()
      .optional(),
    github: z
      .object({
        token: z.string().nullable().optional(),
      })
      .nullable()
      .optional(),
  })
  .strict()

// Settings response schema
export const SettingsResponseSchema = z.object({
  settings: ApplicationSettingsSchema,
  status: SystemStatusCheckSchema,
})

// Derived types
export type ApplicationSettings = z.infer<typeof ApplicationSettingsSchema>
export type SystemStatusCheck = z.infer<typeof SystemStatusCheckSchema>
export type UpdateSettingsSchemaType = z.infer<typeof updateSettingsSchema>
export type SettingsResponse = z.infer<typeof SettingsResponseSchema>
