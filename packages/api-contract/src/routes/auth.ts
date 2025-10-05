import { createRoute } from '@hono/zod-openapi'
import { oauthCallbackQuerySchema, sessionResponseSchema } from '@moatless/types'
import { z } from 'zod'
import { withStandardErrors } from '../responses.js'

export const authLoginRoute = createRoute({
  method: 'get',
  path: '/api/v1/auth/login',
  tags: ['Authentication'],
  description:
    'OAuth callback endpoint - exchanges GitHub OAuth code for session cookie and redirects to frontend',
  request: {
    query: oauthCallbackQuerySchema,
  },
  responses: {
    302: {
      description: 'Redirects to frontend with session cookie set',
    },
    400: {
      description: 'Invalid OAuth state parameter (CSRF protection)',
    },
    500: {
      description: 'Failed to exchange code or create session',
    },
  },
})

export const getCurrentUserRoute = createRoute({
  method: 'get',
  path: '/api/v1/auth/me',
  tags: ['Authentication'],
  description:
    'Get current authenticated user session. In dev mode (GITHUB_OAUTH_ENABLED !== true), returns mock dev user.',
  responses: withStandardErrors(
    sessionResponseSchema,
    'Returns current user session with expiration time',
  ),
})

export const logoutRoute = createRoute({
  method: 'post',
  path: '/api/v1/auth/logout',
  tags: ['Authentication'],
  description: 'Logout and destroy session, clears session cookie',
  responses: withStandardErrors(
    z.object({ message: z.string() }),
    'Returns logout success message',
  ),
})
