import type { RepositoryList } from '@moatless/types'
import {
  CreateRepositorySchema,
  FileSearchSchema,
  RepositoryNotFoundError,
  UpdateRepositorySchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import { z } from 'zod'
import type { AppBindings } from '../app.js'

// Response schemas
const repositoryResponseSchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string().optional(),
  path: z.string(),
  useGitWorktrees: z.boolean().optional(),
  originalBranch: z.string().optional(),
  codeExecution: z.object({
    type: z.enum(['local', 'docker']),
    setupCommands: z.array(z.string()).optional(),
    containerName: z.string().optional(),
    dockerImage: z.string().optional(),
    envVars: z.record(z.string()).optional(),
    volumes: z.array(z.string()).optional(),
    ports: z.record(z.number()).optional(),
    volumeMapping: z
      .object({
        hostPath: z.string(),
        containerPath: z.string(),
      })
      .optional(),
    resources: z
      .object({
        cpuLimit: z.string(),
        memoryLimit: z.string(),
      })
      .optional(),
  }),
  previewConfig: z
    .object({
      framework: z.enum(['vite', 'nextjs', 'cra', 'webpack', 'unknown']).optional(),
      frameworkVersion: z.string().optional(),
      serverStartCommand: z.string().optional(),
      injectionStrategy: z
        .object({
          method: z.enum(['html-injection', 'middleware', 'build-plugin', 'proxy', 'none']),
          config: z.record(z.any()).optional(),
        })
        .optional(),
    })
    .optional(),
  createdAt: z.string(),
  updatedAt: z.string(),
})

const repositoryListResponseSchema = z.object({
  repositories: z.array(repositoryResponseSchema),
})

const fileSearchResultSchema = z.object({
  files: z.array(
    z.object({
      path: z.string(),
      type: z.string(),
      size: z.number().optional(),
    }),
  ),
})

const branchesResponseSchema = z.object({
  branches: z.array(z.string()),
})

const previewConfigResponseSchema = z.object({
  previewConfig: z.object({
    framework: z.string().optional(),
    buildCommand: z.string().optional(),
    devCommand: z.string().optional(),
    startCommand: z.string().optional(),
    installCommand: z.string().optional(),
    port: z.number().optional(),
  }),
})

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
              schema: resolver(repositoryResponseSchema),
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
              schema: resolver(repositoryListResponseSchema),
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

  // Get repository by ID
  app.get(
    `${prefix}/:repositoryId`,
    describeRoute({
      description: 'Get repository by ID',
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
          description: 'Repository details',
          content: {
            'application/json': {
              schema: resolver(repositoryResponseSchema),
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
              schema: resolver(repositoryResponseSchema),
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
              schema: resolver(fileSearchResultSchema),
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
              schema: resolver(branchesResponseSchema),
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

  // Detect framework
  app.post(
    `${prefix}/:repositoryId/detect-framework`,
    describeRoute({
      description: 'Detect framework and preview configuration for repository',
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
          description: 'Detected framework configuration',
          content: {
            'application/json': {
              schema: resolver(previewConfigResponseSchema),
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

      const previewConfig = await repositoryService.detectFramework(repository.path)
      return c.json({ previewConfig })
    },
  )
}
