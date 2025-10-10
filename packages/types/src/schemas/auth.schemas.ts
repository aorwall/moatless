import { z } from 'zod'

/**
 * GitHub OAuth user schema
 */
export const gitHubOAuthUserSchema = z.object({
  id: z.number(),
  login: z.string(),
  email: z.string().email().nullable(),
  name: z.string().nullable(),
  avatar_url: z.string().url().nullable(),
})

/**
 * User session schema
 */
export const userSessionSchema = z.object({
  id: z.string(),
  userId: z.number(),
  userLogin: z.string(),
  userEmail: z.string().email().nullable(),
  userName: z.string().nullable(),
  avatarUrl: z.string().url().nullable(),
  accessToken: z.string(),
  tokenType: z.string(),
  scope: z.string(),
  createdAt: z.date(),
  expiresAt: z.date(),
  lastAccessedAt: z.date(),
})

/**
 * Session response schema
 */
export const sessionResponseSchema = z.object({
  user: z
    .object({
      id: z.number(),
      login: z.string(),
      email: z.string().email().nullable(),
      name: z.string().nullable(),
      avatarUrl: z.string().url().nullable(),
    })
    .nullable(),
  expiresAt: z.string().nullable(),
})

/**
 * GitHub OAuth token response schema
 */
export const gitHubOAuthTokenResponseSchema = z.object({
  access_token: z.string(),
  token_type: z.string(),
  scope: z.string(),
})

/**
 * OAuth callback query parameters schema
 */
export const oauthCallbackQuerySchema = z.object({
  code: z.string(),
  state: z.string().optional(),
})
