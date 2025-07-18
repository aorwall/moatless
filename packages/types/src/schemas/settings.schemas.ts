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

// Derived types
export type ApplicationSettings = z.infer<typeof ApplicationSettingsSchema>
export type SystemStatusCheck = z.infer<typeof SystemStatusCheckSchema>
