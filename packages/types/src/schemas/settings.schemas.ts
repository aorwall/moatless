import { z } from 'zod'
import { ClaudeSettingsSchema } from './settings/claude.schemas.js'
import {
  KubernetesSettingsSchema,
  WorkspaceExecutionModeSchema,
} from './settings/kubernetes.schemas.js'
import { McpServerConfigSchema, McpSettingsSchema } from './settings/mcp.schemas.js'
import { PublishSettingsSchema } from './settings/publish.schemas.js'

export const ApplicationSettingsSchema = z.object({
  dataDir: z.string().optional(),
  repositoriesDir: z.string().optional(),
  defaultWorkspaceType: z.enum(['docker', 'local']).optional(),
  // Workspace execution mode: 'local' or 'kubernetes'
  workspaceExecutionMode: WorkspaceExecutionModeSchema.optional(),
  // Claude settings at root level
  binDir: z.string().optional(),
  configDir: z.string().optional(),
  oauthToken: z.string().optional(),
  // Nested settings
  claude: ClaudeSettingsSchema.optional(),
  kubernetes: KubernetesSettingsSchema.optional(),
  mcp: McpSettingsSchema.optional(),
  publish: PublishSettingsSchema.optional(),
  // User-to-group mappings for group-based access control (userId -> groupId)
  userGroups: z.record(z.string(), z.string()).optional(),
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
  kubernetes: z
    .object({
      isAvailable: z.boolean(),
      currentContext: z.string().nullable(),
      namespace: z.string().nullable(),
      error: z.string().nullable(),
    })
    .optional(),
})

// Update settings schema
export const updateSettingsSchema = z
  .object({
    dataDir: z.string().nullable().optional(),
    repositoriesDir: z.string().nullable().optional(),
    defaultWorkspaceType: z.enum(['docker', 'local']).nullable().optional(),
    workspaceExecutionMode: WorkspaceExecutionModeSchema.nullable().optional(),
    binDir: z.string().nullable().optional(),
    configDir: z.string().nullable().optional(),
    oauthToken: z.string().nullable().optional(),
    claude: z
      .object({
        binDir: z.string().nullable().optional(),
        configDir: z.string().nullable().optional(),
        oauthToken: z.string().nullable().optional(),
      })
      .strict()
      .nullable()
      .optional(),
    kubernetes: z
      .object({
        enabled: z.boolean().nullable().optional(),
        namespace: z.string().nullable().optional(),
        kubeconfig: z.string().nullable().optional(),
        image: z.string().nullable().optional(),
        workerApiUrl: z.string().nullable().optional(),
        workerApiKey: z.string().nullable().optional(),
      })
      .strict()
      .nullable()
      .optional(),
    mcp: z
      .object({
        mcpServers: z.record(z.string(), McpServerConfigSchema).nullable().optional(),
      })
      .strict()
      .nullable()
      .optional(),
    publish: z
      .object({
        netlifyAccessToken: z.string().nullable().optional(),
      })
      .strict()
      .nullable()
      .optional(),
  })
  .strict()

// Settings response schema
export const SettingsResponseSchema = z.object({
  settings: ApplicationSettingsSchema,
  status: SystemStatusCheckSchema,
  defaults: ApplicationSettingsSchema,
})

// Derived types
export type ApplicationSettings = z.infer<typeof ApplicationSettingsSchema>
export type SystemStatusCheck = z.infer<typeof SystemStatusCheckSchema>
export type UpdateSettingsSchemaType = z.infer<typeof updateSettingsSchema>
export type SettingsResponse = z.infer<typeof SettingsResponseSchema>
