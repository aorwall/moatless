import { createRoute } from '@hono/zod-openapi'
import {
  BranchListResponseSchema,
  CreateRemoteRepositoryFromLocalSchema,
  CreateRepositorySchema,
  RepositoryConfigSchema,
  RepositoryListResponseSchema,
  SuggestedRepositoriesResponseSchema,
  UpdateRepositorySchema,
} from '@moatless/types'
import { z } from 'zod'
import { pathParams } from './params.js'

// ============================================================================
// Repository Routes
// ============================================================================

/**
 * POST /api/v1/repositories
 * Create a new repository configuration
 */
export const createRepositoryRoute = createRoute({
  method: 'post',
  path: '/api/v1/repositories',
  tags: ['Repositories'],
  description:
    'Create a new repository configuration for tracking code changes and running AI agents',
  request: {
    body: {
      content: {
        'application/json': {
          schema: CreateRepositorySchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: RepositoryConfigSchema,
        },
      },
    },
  },
})

/**
 * GET /api/v1/repositories
 * List all configured repositories
 */
export const listRepositoriesRoute = createRoute({
  method: 'get',
  path: '/api/v1/repositories',
  tags: ['Repositories'],
  description: 'List all configured repositories (local, remote, and templates)',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: RepositoryListResponseSchema,
        },
      },
    },
  },
})

/**
 * GET /api/v1/repositories/suggestions
 * Get repository suggestions from Claude Code projects
 * IMPORTANT: This must be registered before dynamic routes like /:repositoryId
 */
export const getRepositorySuggestionsRoute = createRoute({
  method: 'get',
  path: '/api/v1/repositories/suggestions',
  tags: ['Repositories'],
  description:
    'Get repository suggestions from Claude Code projects with git repositories on the local machine',
  request: {
    query: z.object({
      onlyGitRepos: z.string().optional(),
    }),
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: SuggestedRepositoriesResponseSchema,
        },
      },
    },
  },
})

/**
 * GET /api/v1/repositories/{repositoryId}
 * Get repository by ID
 */
export const getRepositoryRoute = createRoute({
  method: 'get',
  path: '/api/v1/repositories/{repositoryId}',
  tags: ['Repositories'],
  description: 'Retrieve a specific repository by its unique identifier with full configuration',
  request: {
    params: pathParams.repositoryId,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: RepositoryConfigSchema,
        },
      },
    },
  },
})

/**
 * PUT /api/v1/repositories/{repositoryId}
 * Update repository configuration
 */
export const updateRepositoryRoute = createRoute({
  method: 'put',
  path: '/api/v1/repositories/{repositoryId}',
  tags: ['Repositories'],
  description: 'Update repository configuration including environment settings and preview config',
  request: {
    params: pathParams.repositoryId,
    body: {
      content: {
        'application/json': {
          schema: UpdateRepositorySchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: RepositoryConfigSchema,
        },
      },
    },
  },
})

/**
 * DELETE /api/v1/repositories/{repositoryId}
 * Delete repository
 */
export const deleteRepositoryRoute = createRoute({
  method: 'delete',
  path: '/api/v1/repositories/{repositoryId}',
  tags: ['Repositories'],
  description: 'Delete a repository configuration',
  request: {
    params: pathParams.repositoryId,
  },
  responses: {
    204: {
      description: 'Repository deleted successfully',
    },
  },
})

/**
 * POST /api/v1/repositories/{repositoryId}/convert-to-template
 * Convert a remote repository to a template repository
 */
export const convertToTemplateRoute = createRoute({
  method: 'post',
  path: '/api/v1/repositories/{repositoryId}/convert-to-template',
  tags: ['Repositories'],
  description:
    'Convert a remote repository to a template repository. Templates cannot be used directly for tasks - only for creating new local repositories.',
  request: {
    params: pathParams.repositoryId,
  },
  responses: {
    200: {
      description: 'Repository converted successfully',
      content: {
        'application/json': {
          schema: RepositoryConfigSchema,
        },
      },
    },
  },
})

/**
 * POST /api/v1/repositories/{repositoryId}/create-remote-repository
 * Create a new remote repository (GitHub) from a local repository and push the code
 */
export const createRemoteRepositoryFromLocalRoute = createRoute({
  method: 'post',
  path: '/api/v1/repositories/{repositoryId}/create-remote-repository',
  tags: ['Repositories'],
  description:
    'Create a new GitHub repository from a local repository, push all code, and convert the local repository to a remote repository.',
  request: {
    params: pathParams.repositoryId,
    body: {
      content: {
        'application/json': {
          schema: CreateRemoteRepositoryFromLocalSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Remote repository created successfully',
      content: {
        'application/json': {
          schema: RepositoryConfigSchema,
        },
      },
    },
  },
})

/**
 * GET /api/v1/repositories/{repositoryId}/branches
 * List git branches in repository
 */
export const getRepositoryBranchesRoute = createRoute({
  method: 'get',
  path: '/api/v1/repositories/{repositoryId}/branches',
  tags: ['Repository Operations'],
  description: 'List all git branches in the repository (local and remote)',
  request: {
    params: pathParams.repositoryId,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: BranchListResponseSchema,
        },
      },
    },
  },
})
