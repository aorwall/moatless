import { z } from 'zod'

// GitHub settings schema
export const GitHubSettingsSchema = z.object({
  personalAccessToken: z.string().optional(),
  userName: z.string().optional(),
  userEmail: z.string().optional(),
})

// Update request schema
export const updateGitHubSettingsSchema = z
  .object({
    personalAccessToken: z.string().nullable().optional(),
    userName: z.string().nullable().optional(),
    userEmail: z.string().nullable().optional(),
  })
  .strict()

// Response schema
export const GitHubSettingsResponseSchema = z.object({
  settings: GitHubSettingsSchema,
  defaults: z.object({
    personalAccessToken: z.string(),
    userName: z.string(),
    userEmail: z.string(),
  }),
})

// Derived types
export type GitHubSettings = z.infer<typeof GitHubSettingsSchema>
export type UpdateGitHubSettingsRequest = z.infer<typeof updateGitHubSettingsSchema>
export type GitHubSettingsResponse = z.infer<typeof GitHubSettingsResponseSchema>

// ===== GitHub Service Connection Schemas =====

/**
 * GitHub service connection configuration
 */
export const GitHubServiceConfigSchema = z.object({
  userId: z.string().describe('User ID who owns this token'),
  authMethod: z
    .enum(['oauth', 'pat'])
    .describe('Authentication method: OAuth or Personal Access Token'),
  accessToken: z.string().describe('GitHub access token'),
  tokenType: z.string().optional().describe('Token type (e.g., Bearer)'),
  scope: z.string().optional().describe('OAuth scopes granted'),
  connectedAt: z.string().describe('ISO 8601 timestamp when user connected'),
  expiresAt: z.string().optional().describe('ISO 8601 timestamp when token expires (OAuth only)'),
  githubUserId: z.string().optional().describe('GitHub user ID'),
  githubLogin: z.string().optional().describe('GitHub username'),
})

/**
 * GitHub OAuth URL response for service connection
 */
export const GitHubOAuthUrlResponseSchema = z.object({
  authUrl: z.string().url().describe('GitHub OAuth authorization URL'),
  state: z.string().describe('CSRF protection state parameter'),
})

/**
 * GitHub OAuth state data stored temporarily
 */
export const GitHubOAuthStateDataSchema = z.object({
  state: z.string().describe('OAuth state parameter'),
  userId: z.string().describe('User ID initiating the OAuth flow'),
  createdAt: z.string().describe('ISO 8601 timestamp when state was created'),
})

/**
 * Request to exchange OAuth code for token
 */
export const GitHubOAuthTokenRequestSchema = z.object({
  code: z.string().describe('OAuth authorization code from GitHub'),
  state: z.string().describe('CSRF protection state parameter'),
})

/**
 * GitHub configuration response
 */
export const GitHubConfigResponseSchema = z.object({
  connected: z.boolean().describe('Whether GitHub is connected'),
  oauthEnabled: z.boolean().describe('Whether OAuth is configured on the server'),
  method: z.enum(['github-oauth', 'service-oauth', 'pat']).optional().describe('Connection method'),
  githubUserId: z.string().optional().describe('GitHub user ID'),
  githubLogin: z.string().optional().describe('GitHub username'),
  connectedAt: z.string().optional().describe('ISO 8601 timestamp when connected'),
  expiresAt: z.string().optional().describe('ISO 8601 timestamp when token expires'),
})

/**
 * Request to save Personal Access Token
 */
export const GitHubPATRequestSchema = z.object({
  accessToken: z.string().min(1).describe('GitHub Personal Access Token'),
})

// Type exports
export type GitHubServiceConfig = z.infer<typeof GitHubServiceConfigSchema>
export type GitHubOAuthUrlResponse = z.infer<typeof GitHubOAuthUrlResponseSchema>
export type GitHubOAuthStateData = z.infer<typeof GitHubOAuthStateDataSchema>
export type GitHubOAuthTokenRequest = z.infer<typeof GitHubOAuthTokenRequestSchema>
export type GitHubConfigResponse = z.infer<typeof GitHubConfigResponseSchema>
export type GitHubPATRequest = z.infer<typeof GitHubPATRequestSchema>
