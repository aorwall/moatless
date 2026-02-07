import { createRoute } from '@hono/zod-openapi'
import { oauthCallbackQuerySchema, sessionResponseSchema } from '@moatless/types'
import { z } from 'zod'

export const initiateAuthLoginRoute = createRoute({
  method: 'get',
  path: '/api/v1/auth/login',
  tags: ['Authentication'],
  description:
    'Generic login endpoint that redirects to the configured authentication provider (GitHub or Auth0)',
  responses: {
    302: {
      description: 'Redirects to configured OAuth provider authorization page',
    },
    500: {
      description: 'Invalid AUTH_PROVIDER configuration',
    },
    503: {
      description: 'Authentication is not enabled',
    },
  },
})

export const initiateGitHubOAuthRoute = createRoute({
  method: 'get',
  path: '/api/v1/auth/github',
  tags: ['Authentication'],
  description:
    'Initiates GitHub OAuth flow by redirecting to GitHub authorization page with CSRF protection',
  responses: {
    302: {
      description: 'Redirects to GitHub OAuth authorization page',
    },
    500: {
      description: 'Failed to generate OAuth URL',
    },
  },
})

export const authLoginRoute = createRoute({
  method: 'get',
  path: '/api/v1/auth/github/callback',
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
    'Get current authenticated user session. In dev mode (GITHUB_OAUTH_ENABLED !== true), returns null user and expiresAt.',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: sessionResponseSchema,
        },
      },
    },
  },
})

export const logoutRoute = createRoute({
  method: 'post',
  path: '/api/v1/auth/logout',
  tags: ['Authentication'],
  description:
    'Logout and destroy session, clears session cookie. For Auth0 users, redirects to Auth0 logout endpoint to clear Auth0 session.',
  responses: {
    200: {
      description: 'Success (GitHub users or Auth0 logout URL generation failed)',
      content: {
        'application/json': {
          schema: z.object({ message: z.string() }),
        },
      },
    },
    302: {
      description: 'Redirects to Auth0 logout endpoint (Auth0 users only)',
    },
  },
})

export const logoutGetRoute = createRoute({
  method: 'get',
  path: '/api/v1/auth/logout',
  tags: ['Authentication'],
  description:
    'Logout and destroy session, clears session cookie. For Auth0 users, redirects to Auth0 logout endpoint to clear Auth0 session. GET version for easier browser navigation.',
  responses: {
    200: {
      description: 'Success (GitHub users or Auth0 logout URL generation failed)',
      content: {
        'application/json': {
          schema: z.object({ message: z.string() }),
        },
      },
    },
    302: {
      description: 'Redirects to Auth0 logout endpoint (Auth0 users only)',
    },
  },
})

export const initiateAuth0OAuthRoute = createRoute({
  method: 'get',
  path: '/api/v1/auth/auth0',
  tags: ['Authentication'],
  description:
    'Initiates Auth0 OAuth flow by redirecting to Auth0 authorization page with CSRF protection',
  responses: {
    302: {
      description: 'Redirects to Auth0 OAuth authorization page',
    },
    500: {
      description: 'Failed to generate OAuth URL',
    },
  },
})

export const verifyAuthRoute = createRoute({
  method: 'get',
  path: '/api/v1/auth/verify',
  tags: ['Authentication'],
  description:
    'Verify authentication for Traefik ForwardAuth. Returns 200 if the request has a valid session cookie or API key, 401 otherwise. Used by Traefik middleware to gate access to preview proxied routes.',
  responses: {
    200: {
      description: 'Authenticated',
    },
    401: {
      description: 'Not authenticated',
      content: {
        'application/json': {
          schema: z.object({
            error: z.string(),
            message: z.string(),
          }),
        },
      },
    },
  },
})

export const authAuth0CallbackRoute = createRoute({
  method: 'get',
  path: '/api/v1/auth/auth0/callback',
  tags: ['Authentication'],
  description:
    'OAuth callback endpoint - exchanges Auth0 OAuth code for session cookie and redirects to frontend',
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
