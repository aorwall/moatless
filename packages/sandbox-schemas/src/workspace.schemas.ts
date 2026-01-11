import { z } from 'zod'

/**
 * Repository config schema (subset needed for workspace provisioning)
 */
export const RepositoryConfigSchema = z.object({
  id: z.string(),
  name: z.string(),
  gitUrl: z.string().optional(),
  branch: z.string().optional(),
  codeExecution: z
    .object({
      setupCommands: z.array(z.string()).optional(),
      envVars: z.record(z.string(), z.string()).optional(),
    })
    .optional(),
  previewConfig: z
    .object({
      serverStartCommands: z.array(z.string()).optional(),
      startupTimeoutSeconds: z.number().optional(),
    })
    .optional(),
})

export type RepositoryConfig = z.infer<typeof RepositoryConfigSchema>

/**
 * Workspace provision request schema
 */
export const WorkspaceProvisionRequestSchema = z.object({
  taskId: z.string(),
  gitCloneUrl: z.string().optional(),
  gitBranch: z.string().optional(),
  workspaceDir: z.string().optional(),
  repositoryConfig: RepositoryConfigSchema.optional(),
})

export type WorkspaceProvisionRequest = z.infer<typeof WorkspaceProvisionRequestSchema>

/**
 * Workspace provision status enum
 */
export const WorkspaceProvisionStatusSchema = z.enum(['pending', 'ready', 'failed'])

export type WorkspaceProvisionStatus = z.infer<typeof WorkspaceProvisionStatusSchema>

/**
 * Workspace provision response schema
 */
export const WorkspaceProvisionResponseSchema = z.object({
  success: z.boolean(),
  taskId: z.string(),
  workspaceDir: z.string().optional(),
  status: WorkspaceProvisionStatusSchema.optional(),
  error: z.string().optional(),
})

export type WorkspaceProvisionResponse = z.infer<typeof WorkspaceProvisionResponseSchema>

/**
 * Workspace status response schema
 */
export const WorkspaceStatusResponseSchema = z.object({
  taskId: z.string(),
  workspaceDir: z.string(),
  status: WorkspaceProvisionStatusSchema,
  gitCloneUrl: z.string().optional(),
  gitBranch: z.string().optional(),
  currentBranch: z.string().optional(),
  error: z.string().optional(),
})

export type WorkspaceStatusResponse = z.infer<typeof WorkspaceStatusResponseSchema>
