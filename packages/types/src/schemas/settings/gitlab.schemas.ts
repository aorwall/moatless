import { z } from 'zod'

// ===== GitLab Service Connection Schemas =====

z.object({
  userId: z.string().describe('User ID who owns this token'),
  authMethod: z
    .enum(['oauth', 'pat'])
    .describe('Authentication method: OAuth or Personal Access Token'),
  accessToken: z.string().describe('GitLab access token'),
  tokenType: z.string().optional().describe('Token type (e.g., Bearer)'),
  scope: z.string().optional().describe('OAuth scopes granted'),
  connectedAt: z.string().describe('ISO 8601 timestamp when user connected'),
  expiresAt: z.string().optional().describe('ISO 8601 timestamp when token expires (OAuth only)'),
  gitlabUserId: z.string().optional().describe('GitLab user ID'),
  gitlabUsername: z.string().optional().describe('GitLab username'),
})

/**
 * GitLab configuration response
 */
export const GitLabConfigResponseSchema = z.object({
  connected: z.boolean().describe('Whether GitLab is connected'),
  method: z.enum(['gitlab-oauth', 'pat']).optional().describe('Connection method'),
  gitlabUserId: z.string().optional().describe('GitLab user ID'),
  gitlabUsername: z.string().optional().describe('GitLab username'),
  connectedAt: z.string().optional().describe('ISO 8601 timestamp when connected'),
  expiresAt: z.string().optional().describe('ISO 8601 timestamp when token expires'),
})

/**
 * Request to save Personal Access Token
 */
export const GitLabPATRequestSchema = z.object({
  accessToken: z.string().min(1).describe('GitLab Personal Access Token'),
})

// Type exports
export type GitLabConfigResponse = z.infer<typeof GitLabConfigResponseSchema>
export type GitLabPATRequest = z.infer<typeof GitLabPATRequestSchema>
