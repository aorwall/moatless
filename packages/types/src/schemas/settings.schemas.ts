import { z } from 'zod'

// Base settings schemas
export const ClaudeSettingsSchema = z.object({
  binDir: z.string().optional(),
  configDir: z.string().optional(),
})

export const GitHubSettingsSchema = z.object({
  personalAccessToken: z.string().optional(),
  userName: z.string().optional(),
  userEmail: z.string().optional(),
})

export const ApplicationSettingsSchema = z.object({
  dataDir: z.string().optional(),
  defaultWorkspaceType: z.enum(['docker', 'local']).optional(),
  claude: ClaudeSettingsSchema.optional(),
  github: GitHubSettingsSchema.optional(),
})

// Request schemas
export const updateSettingsSchema = z
  .object({
    dataDir: z.string().nullable().optional(),
    defaultWorkspaceType: z.enum(['docker', 'local']).nullable().optional(),
    claude: z
      .object({
        binDir: z.string().nullable().optional(),
        configDir: z.string().nullable().optional(),
      })
      .strict()
      .optional(),
    github: z
      .object({
        personalAccessToken: z.string().nullable().optional(),
        userName: z.string().nullable().optional(),
        userEmail: z.string().nullable().optional(),
      })
      .strict()
      .optional(),
  })
  .strict()

// Response schemas
export const SettingsResponseSchema = z.object({
  settings: ApplicationSettingsSchema,
  defaults: z.object({
    dataDir: z.string(),
    defaultWorkspaceType: z.enum(['docker', 'local']),
    claude: z.object({
      binDir: z.string(),
      configDir: z.string(),
    }),
    github: z.object({
      personalAccessToken: z.string(),
      userName: z.string(),
      userEmail: z.string(),
    }),
  }),
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

// Derived types
export type UpdateSettingsSchemaType = z.infer<typeof updateSettingsSchema>
export type ApplicationSettings = z.infer<typeof ApplicationSettingsSchema>
export type ClaudeSettings = z.infer<typeof ClaudeSettingsSchema>
export type GitHubSettings = z.infer<typeof GitHubSettingsSchema>
export type SettingsResponse = z.infer<typeof SettingsResponseSchema>
export type SystemStatusCheck = z.infer<typeof SystemStatusCheckSchema>
