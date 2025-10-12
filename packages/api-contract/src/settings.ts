import { createRoute } from '@hono/zod-openapi'
import {
  ClaudeSettingsResponseSchema,
  GeneralSettingsResponseSchema,
  GitHubSettingsResponseSchema,
  McpSettingsResponseSchema,
  PublishSettingsResponseSchema,
  SystemStatusCheckSchema,
  UpdateMcpSettingsRequestSchema,
  UpdatePublishSettingsRequestSchema,
  updateClaudeSettingsSchema,
  updateGeneralSettingsSchema,
  updateGitHubSettingsSchema,
} from '@moatless/types'

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

// GitHub Settings Routes
export const getGitHubSettingsRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/github',
  tags: ['Settings'],
  description: 'Get GitHub integration settings including OAuth and authentication configuration',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: GitHubSettingsResponseSchema,
        },
      },
    },
  },
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
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: GitHubSettingsResponseSchema,
        },
      },
    },
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

// MCP Settings Routes
export const getMcpSettingsRoute = createRoute({
  method: 'get',
  path: '/api/v1/settings/mcp',
  tags: ['Settings'],
  description: 'Get MCP (Model Context Protocol) server configurations',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: McpSettingsResponseSchema,
        },
      },
    },
  },
})

export const updateMcpSettingsRoute = createRoute({
  method: 'put',
  path: '/api/v1/settings/mcp',
  tags: ['Settings'],
  description: 'Update MCP server configurations',
  request: {
    body: {
      content: {
        'application/json': {
          schema: UpdateMcpSettingsRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: McpSettingsResponseSchema,
        },
      },
    },
  },
})
