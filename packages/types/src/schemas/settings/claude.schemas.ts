import { z } from 'zod'

// Claude settings schema
export const ClaudeSettingsSchema = z.object({
  binDir: z.string().optional(),
  configDir: z.string().optional(),
  oauthToken: z.string().optional(),
})

// Update request schema
export const updateClaudeSettingsSchema = z
  .object({
    binDir: z.string().nullable().optional(),
    configDir: z.string().nullable().optional(),
    oauthToken: z.string().nullable().optional(),
  })
  .strict()

// Response schema - includes validation and override status
export const ClaudeSettingsResponseSchema = z.object({
  binDir: z.string(),
  configDir: z.string(),
  oauthToken: z.string(),
  binDirValid: z.boolean(),
  configDirValid: z.boolean(),
  binDirOverridden: z.boolean(),
  configDirOverridden: z.boolean(),
})

// ===== Claude OAuth Connection Schemas =====

/**
 * Claude OAuth URL response for service connection
 */
export const ClaudeOAuthUrlResponseSchema = z.object({
  authUrl: z.string().url().describe('Claude OAuth authorization URL'),
  state: z.string().describe('CSRF protection state parameter (includes code verifier for PKCE)'),
})

/**
 * Request to exchange OAuth code for token
 */
export const ClaudeOAuthTokenRequestSchema = z.object({
  code: z.string().describe('OAuth authorization code from Claude'),
  state: z.string().describe('CSRF protection state parameter (includes code verifier for PKCE)'),
})

/**
 * Claude connection configuration response
 */
export const ClaudeConnectionConfigSchema = z.object({
  connected: z.boolean().describe('Whether Claude is connected'),
  oauthEnabled: z.boolean().describe('Whether OAuth is available (always true for Claude)'),
  method: z.enum(['oauth', 'pat', 'file']).optional().describe('Connection method'),
  claudeUserId: z.string().optional().describe('Claude user ID (if available)'),
  connectedAt: z.string().optional().describe('ISO 8601 timestamp when connected'),
})

/**
 * Request to save Personal Access Token
 */
export const ClaudePATRequestSchema = z.object({
  accessToken: z.string().min(1).describe('Claude Personal Access Token'),
})

// Derived types
export type ClaudeSettings = z.infer<typeof ClaudeSettingsSchema>
export type UpdateClaudeSettingsRequest = z.infer<typeof updateClaudeSettingsSchema>
export type ClaudeSettingsResponse = z.infer<typeof ClaudeSettingsResponseSchema>
export type ClaudeOAuthUrlResponse = z.infer<typeof ClaudeOAuthUrlResponseSchema>
export type ClaudeOAuthTokenRequest = z.infer<typeof ClaudeOAuthTokenRequestSchema>
export type ClaudeConnectionConfig = z.infer<typeof ClaudeConnectionConfigSchema>
export type ClaudePATRequest = z.infer<typeof ClaudePATRequestSchema>
