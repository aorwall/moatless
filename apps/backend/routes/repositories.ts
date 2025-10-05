import type { OpenAPIHono } from '@hono/zod-openapi'
import { createRoute } from '@hono/zod-openapi'
import type { RepositoryList } from '@moatless/types'
import {
  BranchListResponseSchema,
  CommandExecutionOptionsSchema,
  CreateRepositorySchema,
  FileSearchSchema,
  RepositoryConfigSchema,
  RepositoryFileSearchResponseSchema,
  RepositoryListResponseSchema,
  RepositoryNotFoundError,
  SuggestedRepositoriesResponseSchema,
  UpdateRepositorySchema,
  VerificationResponseSchema,
} from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import { z } from 'zod'
import type { AppBindings } from '../app.js'
import {
  createDeleteRoute,
  createGetByIdRoute,
  createGetListRoute,
  createPostRoute,
  pathParams,
  withStandardErrors,
} from '../utils/openapi-helpers.js'

export async function registerRepositoryRoutes(app: OpenAPIHono<AppBindings>, prefix: string) {
  // Create repository
  const createRepositoryRoute = createPostRoute(
    '/api/v1/repositories',
    CreateRepositorySchema,
    RepositoryConfigSchema,
    {
      operation:
        'Create a new repository configuration for tracking code changes and running AI agents',
      request:
        'Repository details including path, name, optional GitHub URL, and environment configuration',
      response: 'Returns created repository with unique ID, configuration settings, and metadata',
    },
    201,
    ['Repositories'],
  )

  app.openapi(createRepositoryRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const validatedBody = c.req.valid('json')

    const repository = await repositoryService.createRepository(validatedBody)
    return c.json(repository)
  })

  // List repositories
  const listRepositoriesRoute = createGetListRoute(
    '/api/v1/repositories',
    z.object({
      includeVibeRepos: z.string().optional(),
    }),
    RepositoryListResponseSchema,
    {
      operation: 'List all configured repositories with optional filtering for vibe repositories',
      response: 'Returns array of repositories with configuration details and metadata',
    },
    ['Repositories'],
  )

  app.openapi(listRepositoriesRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services

    const { includeVibeRepos } = c.req.valid('query')
    const repositories = await repositoryService.listRepositories(includeVibeRepos === 'true')
    return c.json({ repositories } as RepositoryList)
  })

  // Get suggested repositories from Claude projects
  // IMPORTANT: This must be registered before dynamic routes like /:repositoryId
  const getSuggestionsRoute = createRoute({
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
    responses: withStandardErrors(
      SuggestedRepositoriesResponseSchema,
      'Returns list of suggested repositories from Claude Code projects including name, path, and git status',
    ),
  })

  app.openapi(getSuggestionsRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services

    const { onlyGitRepos } = c.req.valid('query')
    const suggestions = await repositoryService.getSuggestedRepositories(onlyGitRepos !== 'false')

    return c.json({ suggestions })
  })

  // Get single repository with all configuration
  const getRepositoryRoute = createGetByIdRoute(
    '/api/v1/repositories/{repositoryId}',
    pathParams.repositoryId,
    RepositoryConfigSchema,
    {
      operation: 'Retrieve a specific repository by its unique identifier with full configuration',
      response:
        'Returns repository configuration including path, environment settings, preview config, and metadata',
    },
    ['Repositories'],
  )

  app.openapi(getRepositoryRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const { repositoryId } = c.req.valid('param')

    const repository = await repositoryService.getRepositoryById(repositoryId)
    if (!repository) {
      throw new RepositoryNotFoundError()
    }

    return c.json(repository)
  })

  // Update repository
  const updateRepositoryRoute = createRoute({
    method: 'put',
    path: '/api/v1/repositories/{repositoryId}',
    tags: ['Repositories'],
    description:
      'Update repository configuration including environment settings and preview config',
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
    responses: withStandardErrors(
      RepositoryConfigSchema,
      'Returns updated repository configuration with modified settings',
    ),
  })

  app.openapi(updateRepositoryRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const { repositoryId } = c.req.valid('param')
    const validatedBody = c.req.valid('json')

    const repository = await repositoryService.updateRepository(repositoryId, validatedBody)
    return c.json(repository)
  })

  // Delete repository
  const deleteRepositoryRoute = createDeleteRoute(
    '/api/v1/repositories/{repositoryId}',
    pathParams.repositoryId,
    'Repository deleted successfully',
    ['Repositories'],
  )

  app.openapi(deleteRepositoryRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const { repositoryId } = c.req.valid('param')

    await repositoryService.deleteRepository(repositoryId)
    return c.body(null, 204)
  })

  // Search files in repository
  const searchFilesRoute = createRoute({
    method: 'post',
    path: '/api/v1/repositories/{repositoryId}/search-files',
    tags: ['Repository Operations'],
    description: 'Search for files in the repository by filename or path pattern',
    request: {
      params: pathParams.repositoryId,
      body: {
        content: {
          'application/json': {
            schema: FileSearchSchema,
          },
        },
      },
    },
    responses: withStandardErrors(
      RepositoryFileSearchResponseSchema,
      'Returns search results with file paths, types, and sizes',
    ),
  })

  app.openapi(searchFilesRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const { repositoryId } = c.req.valid('param')
    const validatedBody = c.req.valid('json')

    const repository = await repositoryService.getRepositoryById(repositoryId)
    if (!repository) {
      throw new RepositoryNotFoundError()
    }

    const results = await repositoryService.searchFiles(repositoryId, validatedBody)
    return c.json(results)
  })

  // Get git branches
  const getBranchesRoute = createRoute({
    method: 'get',
    path: '/api/v1/repositories/{repositoryId}/branches',
    tags: ['Repository Operations'],
    description: 'List all git branches in the repository (local and remote)',
    request: {
      params: pathParams.repositoryId,
    },
    responses: withStandardErrors(
      BranchListResponseSchema,
      'Returns list of git branch names including local and remote branches',
    ),
  })

  app.openapi(getBranchesRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const { repositoryId } = c.req.valid('param')

    const repository = await repositoryService.getRepositoryById(repositoryId)
    if (!repository) {
      throw new RepositoryNotFoundError()
    }

    const branches = await repositoryService.getBranches(repository.path)
    return c.json({ branches })
  })

  // Execute commands in repository with streaming output
  // Note: SSE endpoints don't integrate well with OpenAPI spec, using plain Hono
  app.post(`${prefix}/:repositoryId/execute-commands`, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services

    const repositoryId = c.req.param('repositoryId')
    if (!repositoryId) {
      throw new HTTPException(400, { message: 'Repository ID is required' })
    }

    const body = await c.req.json()
    const options = CommandExecutionOptionsSchema.parse(body)

    // Import SSE utility
    const { createSSEStream } = await import('../utils/sse-stream.js')

    // Create SSE stream from command execution generator
    const shellLogGenerator = repositoryService.executeCommands(repositoryId, options)

    // Transform ShellCommandLogEvent to SSEEvent format
    async function* transformToSSEEvents() {
      for await (const event of shellLogGenerator) {
        yield {
          type: 'shell-log',
          data: {
            logType: event.type,
            timestamp: event.timestamp,
            text: event.text,
          },
        }
      }
    }

    return createSSEStream(transformToSSEEvents(), {
      headers: {
        'Cache-Control': 'no-cache',
        'X-Repository-Id': repositoryId,
      },
    })
  })

  // Verify repository setup
  const verifyRepositoryRoute = createRoute({
    method: 'post',
    path: '/api/v1/repositories/{repositoryId}/verify',
    tags: ['Repository Operations'],
    description:
      'Verify repository setup and configuration including git status, dependencies, and environment',
    request: {
      params: pathParams.repositoryId,
    },
    responses: withStandardErrors(
      VerificationResponseSchema,
      'Returns verification results including setup status, detected issues, and recommendations',
    ),
  })

  app.openapi(verifyRepositoryRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const { repositoryId } = c.req.valid('param')

    const verificationResult = await repositoryService.verifyRepositorySetup(repositoryId)
    return c.json(verificationResult)
  })
}
