import { z } from 'zod'

/**
 * Workspace provisioning modes
 */
export type ProvisionMode = 'direct' | 'worktree' | 'clone'

/**
 * Workspace provision plan
 * Describes how a workspace should be materialized
 */
export const WorkspaceProvisionPlanSchema = z.object({
  mode: z.enum(['direct', 'worktree', 'clone']),
  branch: z.string(),
  reusePath: z.string().optional(),
  localTargetPath: z.string().optional(),
  setupMetadataHash: z.string(),
})

export type WorkspaceProvisionPlan = z.infer<typeof WorkspaceProvisionPlanSchema>

/**
 * Credential bundle for repository access
 * (Future: will support SSH keys, tokens for worker-based execution)
 */
export interface CredentialBundle {
  type: 'none' | 'ssh' | 'token'
  token?: string
  sshKey?: string
}
