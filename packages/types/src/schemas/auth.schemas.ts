import { z } from 'zod'

/**
 * Authentication provider schema
 */
export const authProviderSchema = z.enum(['github', 'auth0'])

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
 * Auth0 user schema
 */
export const auth0UserSchema = z.object({
  sub: z.string(),
  email: z.string().email().nullable(),
  name: z.string().nullable(),
  nickname: z.string().nullable(),
  picture: z.string().url().nullable(),
  email_verified: z.boolean().optional(),
})

/**
 * User session schema
 */
export const userSessionSchema = z.object({
  id: z.string(),
  provider: authProviderSchema,
  userId: z.string(),
  userLogin: z.string(),
  userEmail: z.string().email().nullable(),
  userName: z.string().nullable(),
  avatarUrl: z.string().url().nullable(),
  accessToken: z.string().optional(),
  idToken: z.string().optional(),
  tokenType: z.string().optional(),
  scope: z.string().optional(),
  groupId: z.string().optional(),
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
      id: z.string(),
      login: z.string(),
      email: z.string().email().nullable(),
      name: z.string().nullable(),
      avatarUrl: z.string().url().nullable(),
      provider: authProviderSchema,
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
 * Auth0 OAuth token response schema
 */
export const auth0TokenResponseSchema = z.object({
  access_token: z.string(),
  id_token: z.string(),
  token_type: z.string(),
  expires_in: z.number(),
  scope: z.string().optional(),
})

/**
 * OAuth callback query parameters schema
 */
export const oauthCallbackQuerySchema = z.object({
  code: z.string(),
  state: z.string().optional(),
})
