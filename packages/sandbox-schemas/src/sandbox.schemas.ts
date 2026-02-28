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
 * Sandbox provision request schema
 */
export const SandboxProvisionRequestSchema = z.object({
  taskId: z.string(),
  gitCloneUrl: z.string().optional(),
  gitBranch: z.string().optional(),
  workspaceDir: z.string().optional(),
  repositoryConfig: RepositoryConfigSchema.optional(),
})

export type SandboxProvisionRequest = z.infer<typeof SandboxProvisionRequestSchema>

/**
 * Sandbox provision status enum
 */
export const SandboxProvisionStatusSchema = z.enum(['pending', 'ready', 'failed'])

export type SandboxProvisionStatus = z.infer<typeof SandboxProvisionStatusSchema>

/**
 * Sandbox provision response schema
 */
export const SandboxProvisionResponseSchema = z.object({
  success: z.boolean(),
  taskId: z.string(),
  workspaceDir: z.string().optional(),
  status: SandboxProvisionStatusSchema.optional(),
  error: z.string().optional(),
})

export type SandboxProvisionResponse = z.infer<typeof SandboxProvisionResponseSchema>

/**
 * Sandbox status response schema
 */
export const SandboxStatusResponseSchema = z.object({
  taskId: z.string(),
  workspaceDir: z.string(),
  status: SandboxProvisionStatusSchema,
  gitCloneUrl: z.string().optional(),
  gitBranch: z.string().optional(),
  currentBranch: z.string().optional(),
  error: z.string().optional(),
})

export type SandboxStatusResponse = z.infer<typeof SandboxStatusResponseSchema>
