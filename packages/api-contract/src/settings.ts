import { createRoute } from '@hono/zod-openapi'
import {
  ClaudeConnectionConfigSchema,
  ClaudeOAuthUrlResponseSchema,
  ClaudePATRequestSchema,
  ClaudeSettingsResponseSchema,
  GeneralSettingsResponseSchema,
  GitHubConfigResponseSchema,
  GitHubOAuthTokenRequestSchema,
  GitHubOAuthUrlResponseSchema,
  GitHubPATRequestSchema,
  GitLabConfigResponseSchema,
  GitLabPATRequestSchema,
  GitnessConfigResponseSchema,
  GitnessPATRequestSchema,
  PublishSettingsResponseSchema,
  SystemStatusCheckSchema,
  UpdatePublishSettingsRequestSchema,
  updateClaudeSettingsSchema,
  updateGeneralSettingsSchema,
} from '@moatless/types'
import { z } from 'zod'

// GET /api/v1/settings/status - Get system status
export const getSystemStatusRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/status',
  tags: ['Settings'],
  description: 'Retrieve system status checks for all configured tools and services',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: SystemStatusCheckSchema,
        },
      },
    },
  },
})

// General Settings Routes
export const getGeneralSettingsRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/general',
  tags: ['Settings'],
  description:
    'Get general application settings including data directory and default configurations',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: GeneralSettingsResponseSchema,
        },
      },
    },
  },
})

export const updateGeneralSettingsRoute = createRoute({
  method: 'put',
  path: '/api/v1/settings/general',
  tags: ['Settings'],
  description: 'Update general application settings',
  request: {
    body: {
      content: {
        'application/json': {
          schema: updateGeneralSettingsSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: GeneralSettingsResponseSchema,
        },
      },
    },
  },
})

// Claude Settings Routes
export const getClaudeSettingsRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/claude',
  tags: ['Settings'],
  description: 'Get Claude AI settings including API configuration and model preferences',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: ClaudeSettingsResponseSchema,
        },
      },
    },
  },
})

export const updateClaudeSettingsRoute = createRoute({
  method: 'put',
  path: '/api/v1/settings/claude',
  tags: ['Settings'],
  description: 'Update Claude AI settings',
  request: {
    body: {
      content: {
        'application/json': {
          schema: updateClaudeSettingsSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: ClaudeSettingsResponseSchema,
        },
      },
    },
  },
})

// Claude OAuth Connection Routes
export const getClaudeConfigRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/claude/config',
  tags: ['Settings'],
  description:
    "Get current user's Claude connection configuration (OAuth, PAT, or file-based token)",
  responses: {
    200: {
      description: 'Configuration retrieved successfully',
      content: {
        'application/json': {
          schema: ClaudeConnectionConfigSchema,
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

export const getClaudeOAuthUrlRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/claude/oauth/url',
  tags: ['Settings'],
  description:
    'Generate Claude OAuth authorization URL with PKCE. Opens popup for user to authorize and copy token.',
  responses: {
    200: {
      description: 'OAuth URL generated successfully',
      content: {
        'application/json': {
          schema: ClaudeOAuthUrlResponseSchema,
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Failed to generate OAuth URL' },
  },
})

export const deleteClaudeConfigRoute = createRoute({
  method: 'delete',
  path: '/api/v1/settings/claude/config',
  tags: ['Settings'],
  description:
    'Disconnect Claude service connection for the current user by deleting stored tokens',
  responses: {
    200: {
      description: 'Claude connection removed successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

export const saveClaudePATRoute = createRoute({
  method: 'put',
  path: '/api/v1/settings/claude/pat',
  tags: ['Settings'],
  description: 'Save a Claude Personal Access Token for service connection',
  request: {
    body: {
      content: {
        'application/json': {
          schema: ClaudePATRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Personal Access Token saved successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    400: { description: 'Invalid token' },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

// GitHub Service Connection Routes
export const getGitHubConfigRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/github/config',
  tags: ['Settings'],
  description:
    "Get current user's GitHub connection configuration (OAuth or PAT, connection status)",
  responses: {
    200: {
      description: 'Configuration retrieved successfully',
      content: {
        'application/json': {
          schema: GitHubConfigResponseSchema,
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

export const getGitHubOAuthUrlRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/github/oauth/url',
  tags: ['Settings'],
  description:
    'Generate GitHub OAuth authorization URL for service connection. Requires authentication.',
  responses: {
    200: {
      description: 'OAuth URL generated successfully',
      content: {
        'application/json': {
          schema: GitHubOAuthUrlResponseSchema,
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Failed to generate OAuth URL or missing GitHub OAuth configuration' },
  },
})

export const exchangeGitHubTokenRoute = createRoute({
  method: 'post',
  path: '/api/v1/settings/github/oauth/token',
  tags: ['Settings'],
  description:
    'Exchange GitHub OAuth authorization code for access token. Stores token for authenticated user.',
  request: {
    body: {
      content: {
        'application/json': {
          schema: GitHubOAuthTokenRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Token exchanged and saved successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    400: { description: 'Invalid code or state parameter' },
    401: { description: 'Not authenticated' },
    500: { description: 'Failed to exchange token with GitHub API' },
  },
})

export const deleteGitHubConfigRoute = createRoute({
  method: 'delete',
  path: '/api/v1/settings/github/config',
  tags: ['Settings'],
  description:
    'Disconnect GitHub service connection for the current user by deleting stored tokens',
  responses: {
    200: {
      description: 'GitHub connection removed successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

export const saveGitHubPATRoute = createRoute({
  method: 'put',
  path: '/api/v1/settings/github/pat',
  tags: ['Settings'],
  description: 'Save a GitHub Personal Access Token for service connection',
  request: {
    body: {
      content: {
        'application/json': {
          schema: GitHubPATRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Personal Access Token saved successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    400: { description: 'Invalid token' },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

// GitLab Service Connection Routes
export const getGitLabConfigRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/gitlab/config',
  tags: ['Settings'],
  description:
    "Get current user's GitLab connection configuration (OAuth or PAT, connection status)",
  responses: {
    200: {
      description: 'Configuration retrieved successfully',
      content: {
        'application/json': {
          schema: GitLabConfigResponseSchema,
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

export const deleteGitLabConfigRoute = createRoute({
  method: 'delete',
  path: '/api/v1/settings/gitlab/config',
  tags: ['Settings'],
  description:
    'Disconnect GitLab service connection for the current user by deleting stored tokens',
  responses: {
    200: {
      description: 'GitLab connection removed successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

export const saveGitLabPATRoute = createRoute({
  method: 'put',
  path: '/api/v1/settings/gitlab/pat',
  tags: ['Settings'],
  description: 'Save a GitLab Personal Access Token for service connection',
  request: {
    body: {
      content: {
        'application/json': {
          schema: GitLabPATRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Personal Access Token saved successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    400: { description: 'Invalid token' },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

// Gitness Service Connection Routes
export const getGitnessConfigRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/gitness/config',
  tags: ['Settings'],
  description:
    "Get current user's Gitness connection configuration (PAT, connection status, base URL)",
  responses: {
    200: {
      description: 'Configuration retrieved successfully',
      content: {
        'application/json': {
          schema: GitnessConfigResponseSchema,
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

export const deleteGitnessConfigRoute = createRoute({
  method: 'delete',
  path: '/api/v1/settings/gitness/config',
  tags: ['Settings'],
  description:
    'Disconnect Gitness service connection for the current user by deleting stored tokens',
  responses: {
    200: {
      description: 'Gitness connection removed successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

export const saveGitnessPATRoute = createRoute({
  method: 'put',
  path: '/api/v1/settings/gitness/pat',
  tags: ['Settings'],
  description: 'Save a Gitness Personal Access Token with base URL for service connection',
  request: {
    body: {
      content: {
        'application/json': {
          schema: GitnessPATRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Personal Access Token saved successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    400: { description: 'Invalid token or base URL' },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

// Publish Settings Routes
export const getPublishSettingsRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/publish',
  tags: ['Settings'],
  description: 'Get publish/deployment settings including platform configurations',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: PublishSettingsResponseSchema,
        },
      },
    },
  },
})

export const updatePublishSettingsRoute = createRoute({
  method: 'put',
  path: '/api/v1/settings/publish',
  tags: ['Settings'],
  description: 'Update publish/deployment settings',
  request: {
    body: {
      content: {
        'application/json': {
          schema: UpdatePublishSettingsRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: PublishSettingsResponseSchema,
        },
      },
    },
  },
})
