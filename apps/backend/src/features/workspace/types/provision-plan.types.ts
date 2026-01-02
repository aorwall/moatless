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
  branch: z.string().optional(), // Branch to work on (workspace branch)
  targetBranch: z.string().optional(), // Branch to push checkpoints to
  reusePath: z.string().optional(), // Path to clone from (for local/template repos)
  remoteUrl: z.string().optional(), // URL to clone from (for remote repos)
  provider: z.enum(['github', 'gitlab']).optional(), // Git provider type (for remote repos)
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
