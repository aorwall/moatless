import type { FastifyInstance } from 'fastify'
import type { z } from 'zod'
import { EnvironmentNotFoundError } from '../../shared/constants/errors.js'
import {
  CreateEnvironmentSchema,
  EnvironmentIdParamsSchema,
  FileSearchSchema,
  UpdateEnvironmentSchema,
} from '../../shared/schemas/environment.schemas.js'
import type {
  BranchListResponse,
  EnvironmentList,
  FileSearchResponse,
  SetupScriptResponse,
} from '../../shared/types/environment.js'

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

      const environment = await environmentService.createEnvironment(body)
      return environment
    },
  )

  // List environments
  app.get('/', async (_request, _reply) => {
    const environments = await environmentService.listEnvironments()
    return { environments } as EnvironmentList
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
  app.put(
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

  // Run setup script
  app.post(
    '/:environmentId/run-setup',
    {
      schema: {
        params: EnvironmentIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { environmentId } = request.params as z.infer<typeof EnvironmentIdParamsSchema>

      // Verify environment exists
      const environment = await environmentService.getEnvironment(environmentId)
      if (!environment) {
        throw new EnvironmentNotFoundError(environmentId)
      }

      // TODO: Implement setup script execution
      return {
        success: true,
        output: 'Setup script execution not yet implemented',
      } as SetupScriptResponse
    },
  )

  // Search files in environment
  app.post(
    '/:environmentId/search-files',
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
      } as FileSearchResponse
    },
  )

  // Get branches
  app.get(
    '/:environmentId/branches',
    {
      schema: {
        params: EnvironmentIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { environmentId } = request.params as z.infer<typeof EnvironmentIdParamsSchema>

      // Verify environment exists
      const environment = await environmentService.getEnvironment(environmentId)
      if (!environment) {
        throw new EnvironmentNotFoundError(environmentId)
      }

      // TODO: Implement branch listing when workspace service is complete
      return {
        branches: [],
        currentBranch: undefined,
      } as BranchListResponse
    },
  )
}
