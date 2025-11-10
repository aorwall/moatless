import { z } from 'zod'

/**
 * Authentication provider schema
 * Can be any string (e.g., 'github', 'auth0', 'google', 'okta')
 * This allows the system to support any OAuth provider without code changes
 */
export const authProviderSchema = z.string()

/**
 * Session response schema
 */
export const sessionResponseSchema = z.object({
  user: z
    .object({
      id: z.string(),
      login: z.string(),
      email: z.email().nullable(),
      name: z.string().nullable(),
      avatarUrl: z.url().nullable(),
      provider: authProviderSchema,
    })
    .nullable(),
  expiresAt: z.string().nullable(),
})

/**
 * OAuth callback query parameters schema
 */
export const oauthCallbackQuerySchema = z.object({
  code: z.string(),
  state: z.string().optional(),
})
