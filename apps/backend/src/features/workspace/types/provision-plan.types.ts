import { z } from 'zod'

/**
 * Workspace provision plan
 * Describes how a workspace should be materialized (always via git clone)
 */
export const WorkspaceProvisionPlanSchema = z.object({
  branch: z.string().optional(), // Branch to work on (workspace branch)
  targetBranch: z.string().optional(), // Branch to push checkpoints to
  remoteUrl: z.string(), // URL to clone from
  provider: z.enum(['github', 'gitlab']).optional(), // Git provider type
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
