import { createRoute } from '@hono/zod-openapi'
import {
  CurrentCommitHashResponseSchema,
  GitChangesResponseSchema,
  GitDiffResponseSchema,
  GitStatusResponseSchema,
} from '@moatless/sandbox-schemas'
import { SANDBOX_ENDPOINTS } from '../endpoints'

export const gitStatusRoute = createRoute({
  method: 'get',
  path: SANDBOX_ENDPOINTS.GIT_STATUS,
  tags: ['Git'],
  description: 'Get git status for the workspace',
  responses: {
    200: {
      description: 'Git status',
      content: {
        'application/json': {
          schema: GitStatusResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to get git status',
      content: {
        'application/json': {
          schema: GitStatusResponseSchema,
        },
      },
    },
  },
})

export const gitChangesRoute = createRoute({
  method: 'get',
  path: SANDBOX_ENDPOINTS.GIT_CHANGES,
  tags: ['Git'],
  description: 'Get git changes for the workspace',
  responses: {
    200: {
      description: 'Git changes',
      content: {
        'application/json': {
          schema: GitChangesResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to get git changes',
      content: {
        'application/json': {
          schema: GitChangesResponseSchema,
        },
      },
    },
  },
})

export const gitDiffRoute = createRoute({
  method: 'get',
  path: SANDBOX_ENDPOINTS.GIT_DIFF,
  tags: ['Git'],
  description: 'Get git diff for the workspace',
  responses: {
    200: {
      description: 'Git diff',
      content: {
        'application/json': {
          schema: GitDiffResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to get git diff',
      content: {
        'application/json': {
          schema: GitDiffResponseSchema,
        },
      },
    },
  },
})

export const currentCommitHashRoute = createRoute({
  method: 'get',
  path: SANDBOX_ENDPOINTS.CURRENT_COMMIT_HASH,
  tags: ['Git'],
  description: 'Get the current commit hash',
  responses: {
    200: {
      description: 'Current commit hash',
      content: {
        'application/json': {
          schema: CurrentCommitHashResponseSchema,
        },
      },
    },
  },
})
