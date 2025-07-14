import { z } from 'zod'

// Base settings schemas
export const ClaudeSettingsSchema = z.object({
  binDir: z.string().optional(),
  configDir: z.string().optional(),
})

export const ApplicationSettingsSchema = z.object({
  dataDir: z.string().optional(),
  defaultWorkspaceType: z.enum(['docker', 'local']).optional(),
  claude: ClaudeSettingsSchema.optional(),
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
export type SettingsResponse = z.infer<typeof SettingsResponseSchema>
export type SystemStatusCheck = z.infer<typeof SystemStatusCheckSchema>
