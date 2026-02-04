import type { RepositoryConfig } from '@moatless/types'
import { z } from 'zod'

/**
 * Workspace provision plan (hashable/deterministic subset)
 * Describes how a workspace should be materialized (always via git clone)
 */
export const WorkspaceProvisionPlanSchema = z.object({
  remoteUrl: z.string(), // URL to clone from
  provider: z.enum(['github', 'gitlab']).optional(), // Git provider type
  setupMetadataHash: z.string(),
  pushEnabled: z.boolean().default(true), // Control push access (false for template mode)
})

/**
 * Full provision plan passed to provisioners.
 * Extends the Zod schema fields with resolved runtime data (repoConfig, credentials).
 */
export type WorkspaceProvisionPlan = z.infer<typeof WorkspaceProvisionPlanSchema> & {
  repoConfig: RepositoryConfig
  gitCloneUrl: string
  gitBranch: string | undefined
  gitProviderToken: string
  claudeOAuthToken: string | null
  anthropicApiKey: string | undefined
}
