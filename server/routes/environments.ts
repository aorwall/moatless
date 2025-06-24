import { randomUUID } from 'crypto'
import type { FastifyInstance } from 'fastify'
import { z } from 'zod'
import { EnvironmentNotFoundError } from '../../shared/constants/errors.js'

// Schema for URL params
const EnvironmentIdParamsSchema = z.object({
  environmentId: z.string(),
})

// Schema for environment creation
const CreateEnvironmentSchema = z.object({
  name: z.string(),
  description: z.string().optional(),
  repository: z.object({
    type: z.enum(['local', 'github', 'gitlab']),
    path: z.string().optional(),
    url: z.string().optional(),
    branch: z.string().optional(),
    commit: z.string().optional(),
    repoId: z.string().optional(),
    token: z.string().optional(),
  }),
  codeExecution: z.object({
    type: z.enum(['local', 'docker', 'sandbox']),
    setupCommands: z.string().optional(),
    serverCommand: z.string().optional(),
    serverPortPlaceholder: z.string().optional(),
    image: z.string().optional(),
    envVars: z.record(z.string()).optional(),
    volumes: z.array(z.string()).optional(),
    ports: z.record(z.number()).optional(),
  }),
  variables: z.record(z.string()).optional(),
})

// Schema for environment update
const UpdateEnvironmentSchema = z.object({
  name: z.string().optional(),
  description: z.string().optional(),
  isActive: z.boolean().optional(),
})

// Schema for file search
const FileSearchSchema = z.object({
  query: z.string().min(1),
  filePattern: z.string().optional(),
  maxResults: z.number().min(1).max(1000).default(100),
})

export async function registerEnvironmentRoutes(app: FastifyInstance) {
  const { environment: environmentService } = app.services

  // Create environment
  app.post(
    '/',
    {
      schema: {
        body: CreateEnvironmentSchema,
      },
    },
    async (request, _reply) => {
      const body = request.body as z.infer<typeof CreateEnvironmentSchema>
      const environmentData = {
        ...body,
        id: randomUUID(),
        createdAt: new Date(),
        updatedAt: new Date(),
        isActive: true,
        variables: body.variables || {},
      }

      await environmentService.createEnvironment(environmentData as any)
      return environmentData
    },
  )

  // List environments
  app.get('/', async (_request, _reply) => {
    const environments = await environmentService.listEnvironments()
    return { environments }
  })

  // Get environment by ID
  app.get(
    '/:environmentId',
    {
      schema: {
        params: EnvironmentIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { environmentId } = request.params as z.infer<typeof EnvironmentIdParamsSchema>
      const environment = await environmentService.getEnvironment(environmentId)

      if (!environment) {
        throw new EnvironmentNotFoundError(environmentId)
      }

      return environment
    },
  )

  // Update environment
  app.patch(
    '/:environmentId',
    {
      schema: {
        params: EnvironmentIdParamsSchema,
        body: UpdateEnvironmentSchema,
      },
    },
    async (request, _reply) => {
      const { environmentId } = request.params as z.infer<typeof EnvironmentIdParamsSchema>
      const updates = request.body as z.infer<typeof UpdateEnvironmentSchema>

      await environmentService.updateEnvironment(environmentId, updates)

      const updatedEnvironment = await environmentService.getEnvironment(environmentId)
      return updatedEnvironment
    },
  )

  // Delete environment
  app.delete(
    '/:environmentId',
    {
      schema: {
        params: EnvironmentIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { environmentId } = request.params as z.infer<typeof EnvironmentIdParamsSchema>

      await environmentService.deleteEnvironment(environmentId)
      return { status: 'deleted' }
    },
  )

  // Search files in environment
  app.post(
    '/:environmentId/search',
    {
      schema: {
        params: EnvironmentIdParamsSchema,
        body: FileSearchSchema,
      },
    },
    async (request, _reply) => {
      const { environmentId } = request.params as z.infer<typeof EnvironmentIdParamsSchema>
      // TODO: Use searchRequest when implementing file search
      // const searchRequest = request.body as z.infer<typeof FileSearchSchema>

      // Verify environment exists
      const environment = await environmentService.getEnvironment(environmentId)
      if (!environment) {
        throw new EnvironmentNotFoundError(environmentId)
      }

      // TODO: Implement file search when workspace service is complete
      return {
        files: [],
        totalCount: 0,
        truncated: false,
      }
    },
  )
}
