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
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../app.js'

export async function registerRepositoryRoutes(app: Hono<AppBindings>, prefix: string) {
  // Create repository
  app.post(
    prefix,
    describeRoute({
      description: 'Create a new repository',
      requestBody: {
        description: 'Repository data to create',
        content: {
          'application/json': {
            schema: resolver(CreateRepositorySchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Repository created successfully',
          content: {
            'application/json': {
              schema: resolver(RepositoryConfigSchema),
            },
          },
        },
        400: {
          description: 'Invalid request body',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { repository: repositoryService } = services

      const body = await c.req.json()
      const validatedBody = CreateRepositorySchema.parse(body)

      const repository = await repositoryService.createRepository(validatedBody)
      return c.json(repository)
    },
  )

  // List repositories
  app.get(
    prefix,
    describeRoute({
      description: 'Get list of all repositories',
      responses: {
        200: {
          description: 'List of repositories',
          content: {
            'application/json': {
              schema: resolver(RepositoryListResponseSchema),
            },
          },
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { repository: repositoryService } = services

      const repositories = await repositoryService.listRepositories()
      return c.json({ repositories } as RepositoryList)
    },
  )

  // Get suggested repositories from Claude projects
  // IMPORTANT: This must be registered before dynamic routes like /:repositoryId
  app.get(
    `${prefix}/suggestions`,
    describeRoute({
      description: 'Get suggested repositories from Claude projects',
      parameters: [
        {
          name: 'onlyGitRepos',
          in: 'query',
          required: false,
          description: 'Filter to only show git repositories (exclude worktrees)',
          schema: {
            type: 'boolean',
          },
        },
      ],
      responses: {
        200: {
          description: 'List of suggested repositories',
          content: {
            'application/json': {
              schema: resolver(SuggestedRepositoriesResponseSchema),
            },
          },
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { repository: repositoryService } = services

      const onlyGitRepos = c.req.query('onlyGitRepos') !== 'false' // Default to true
      const suggestions = await repositoryService.getSuggestedRepositories(onlyGitRepos)

      return c.json({ suggestions })
    },
  )

  // Get single repository with all configuration
  app.get(
    `${prefix}/:repositoryId`,
    describeRoute({
      description: 'Get a single repository with all its configuration',
      parameters: [
        {
          name: 'repositoryId',
          in: 'path',
          required: true,
          description: 'Repository ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Repository details with all configuration',
          content: {
            'application/json': {
              schema: resolver(RepositoryConfigSchema),
            },
          },
        },
        404: {
          description: 'Repository not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { repository: repositoryService } = services

      const repositoryId = c.req.param('repositoryId')
      if (!repositoryId) {
        throw new HTTPException(400, { message: 'Repository ID is required' })
      }

      const repository = await repositoryService.getRepositoryById(repositoryId)
      if (!repository) {
        throw new RepositoryNotFoundError()
      }

      return c.json(repository)
    },
  )

  // Update repository
  app.put(
    `${prefix}/:repositoryId`,
    describeRoute({
      description: 'Update repository',
      parameters: [
        {
          name: 'repositoryId',
          in: 'path',
          required: true,
          description: 'Repository ID',
          schema: {
            type: 'string',
          },
        },
      ],
      requestBody: {
        description: 'Repository data to update',
        content: {
          'application/json': {
            schema: resolver(UpdateRepositorySchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Repository updated successfully',
          content: {
            'application/json': {
              schema: resolver(RepositoryConfigSchema),
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        404: {
          description: 'Repository not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { repository: repositoryService } = services

      const repositoryId = c.req.param('repositoryId')
      if (!repositoryId) {
        throw new HTTPException(400, { message: 'Repository ID is required' })
      }

      const body = await c.req.json()
      const validatedBody = UpdateRepositorySchema.parse(body)

      const repository = await repositoryService.updateRepository(repositoryId, validatedBody)
      return c.json(repository)
    },
  )

  // Delete repository
  app.delete(
    `${prefix}/:repositoryId`,
    describeRoute({
      description: 'Delete repository',
      parameters: [
        {
          name: 'repositoryId',
          in: 'path',
          required: true,
          description: 'Repository ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        204: {
          description: 'Repository deleted successfully',
        },
        400: {
          description: 'Invalid repository ID',
        },
        404: {
          description: 'Repository not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { repository: repositoryService } = services

      const repositoryId = c.req.param('repositoryId')
      if (!repositoryId) {
        throw new HTTPException(400, { message: 'Repository ID is required' })
      }

      await repositoryService.deleteRepository(repositoryId)
      return c.body(null, 204)
    },
  )

  // Search files in repository
  app.post(
    `${prefix}/:repositoryId/search-files`,
    describeRoute({
      description: 'Search files in repository',
      parameters: [
        {
          name: 'repositoryId',
          in: 'path',
          required: true,
          description: 'Repository ID',
          schema: {
            type: 'string',
          },
        },
      ],
      requestBody: {
        description: 'File search parameters',
        content: {
          'application/json': {
            schema: resolver(FileSearchSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'File search results',
          content: {
            'application/json': {
              schema: resolver(RepositoryFileSearchResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        404: {
          description: 'Repository not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { repository: repositoryService } = services

      const repositoryId = c.req.param('repositoryId')
      if (!repositoryId) {
        throw new HTTPException(400, { message: 'Repository ID is required' })
      }

      const body = await c.req.json()
      const validatedBody = FileSearchSchema.parse(body)

      const repository = await repositoryService.getRepositoryById(repositoryId)
      if (!repository) {
        throw new RepositoryNotFoundError()
      }

      // File search is only supported for repositories with a path

      const results = await repositoryService.searchFiles(repositoryId, validatedBody)
      return c.json(results)
    },
  )

  // Get git branches
  app.get(
    `${prefix}/:repositoryId/branches`,
    describeRoute({
      description: 'Get git branches for repository',
      parameters: [
        {
          name: 'repositoryId',
          in: 'path',
          required: true,
          description: 'Repository ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'List of git branches',
          content: {
            'application/json': {
              schema: resolver(BranchListResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid repository ID',
        },
        404: {
          description: 'Repository not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { repository: repositoryService } = services

      const repositoryId = c.req.param('repositoryId')
      if (!repositoryId) {
        throw new HTTPException(400, { message: 'Repository ID is required' })
      }

      const repository = await repositoryService.getRepositoryById(repositoryId)

      if (!repository) {
        throw new RepositoryNotFoundError()
      }

      const branches = await repositoryService.getBranches(repository.path)
      return c.json({ branches })
    },
  )

  // Execute commands in repository with streaming output
  app.post(
    `${prefix}/:repositoryId/execute-commands`,
    describeRoute({
      description: 'Execute commands in a repository workspace with streaming output',
      parameters: [
        {
          name: 'repositoryId',
          in: 'path',
          required: true,
          description: 'Repository ID',
          schema: {
            type: 'string',
          },
        },
      ],
      requestBody: {
        description: 'Command execution options',
        content: {
          'application/json': {
            schema: resolver(CommandExecutionOptionsSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'SSE stream of command execution events',
          content: {
            'text/event-stream': {
              schema: {
                type: 'string',
                description: 'Server-sent events stream',
              },
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        404: {
          description: 'Repository not found',
        },
      },
    }),
    async (c) => {
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
      const generator = repositoryService.executeCommands(repositoryId, options)

      return createSSEStream(generator, {
        headers: {
          'Cache-Control': 'no-cache',
          'X-Repository-Id': repositoryId,
        },
      })
    },
  )

  // Verify repository setup
  app.post(
    `${prefix}/:repositoryId/verify`,
    describeRoute({
      description: 'Verify repository setup by testing workspace creation and setup commands',
      parameters: [
        {
          name: 'repositoryId',
          in: 'path',
          required: true,
          description: 'Repository ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Verification results',
          content: {
            'application/json': {
              schema: resolver(VerificationResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid repository ID',
        },
        404: {
          description: 'Repository not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { repository: repositoryService } = services

      const repositoryId = c.req.param('repositoryId')
      if (!repositoryId) {
        throw new HTTPException(400, { message: 'Repository ID is required' })
      }

      const verificationResult = await repositoryService.verifyRepositorySetup(repositoryId)
      return c.json(verificationResult)
    },
  )
}
