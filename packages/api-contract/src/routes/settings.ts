import { createRoute } from '@hono/zod-openapi'
import {
  ClaudeSettingsResponseSchema,
  GeneralSettingsResponseSchema,
  GitHubSettingsResponseSchema,
  PublishSettingsResponseSchema,
  SystemStatusCheckSchema,
  UpdatePublishSettingsRequestSchema,
  updateClaudeSettingsSchema,
  updateGeneralSettingsSchema,
  updateGitHubSettingsSchema,
} from '@moatless/types'
import { withStandardErrors } from '../responses.js'

// GET /api/v1/settings/status - Get system status
export const getSystemStatusRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/status',
  tags: ['Settings'],
  description: 'Retrieve system status checks for all configured tools and services',
  responses: withStandardErrors(
    SystemStatusCheckSchema,
    'System status retrieved successfully with validation results',
  ),
})

// General Settings Routes
export const getGeneralSettingsRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/general',
  tags: ['Settings'],
  description:
    'Get general application settings including data directory and default configurations',
  responses: withStandardErrors(
    GeneralSettingsResponseSchema,
    'Returns general settings configuration',
  ),
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
  responses: withStandardErrors(GeneralSettingsResponseSchema, 'Returns updated general settings'),
})

// Claude Settings Routes
export const getClaudeSettingsRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/claude',
  tags: ['Settings'],
  description: 'Get Claude AI settings including API configuration and model preferences',
  responses: withStandardErrors(
    ClaudeSettingsResponseSchema,
    'Returns Claude settings configuration',
  ),
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
  responses: withStandardErrors(ClaudeSettingsResponseSchema, 'Returns updated Claude settings'),
})

// GitHub Settings Routes
export const getGitHubSettingsRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/github',
  tags: ['Settings'],
  description: 'Get GitHub integration settings including OAuth and authentication configuration',
  responses: withStandardErrors(
    GitHubSettingsResponseSchema,
    'Returns GitHub settings configuration',
  ),
})

export const updateGitHubSettingsRoute = createRoute({
  method: 'put',
  path: '/api/v1/settings/github',
  tags: ['Settings'],
  description: 'Update GitHub integration settings',
  request: {
    body: {
      content: {
        'application/json': {
          schema: updateGitHubSettingsSchema,
        },
      },
    },
  },
  responses: withStandardErrors(GitHubSettingsResponseSchema, 'Returns updated GitHub settings'),
})

// Publish Settings Routes
export const getPublishSettingsRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/publish',
  tags: ['Settings'],
  description: 'Get publish/deployment settings including platform configurations',
  responses: withStandardErrors(
    PublishSettingsResponseSchema,
    'Returns publish settings configuration',
  ),
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
  responses: withStandardErrors(PublishSettingsResponseSchema, 'Returns updated publish settings'),
})
