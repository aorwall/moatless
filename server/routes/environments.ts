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
  SetupScriptResponse,
} from '../../shared/types/environment.js'
import { searchFilesInDirectoryNew } from '../utils/file-search.js'

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
      if (!updatedEnvironment) {
        throw new EnvironmentNotFoundError(environmentId)
      }
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
      const searchRequest = request.body as z.infer<typeof FileSearchSchema>

      // Verify environment exists
      const environment = await environmentService.getEnvironment(environmentId)
      if (!environment) {
        throw new EnvironmentNotFoundError(environmentId)
      }

      // Only support local repositories for now
      if (environment.repository.type !== 'local') {
        throw new Error('File search is only supported for local repositories')
      }

      // Search files in the repository path
      const result = await searchFilesInDirectoryNew(environment.repository.path, {
        query: searchRequest.query,
        limit: searchRequest.limit,
      })

      return result
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

  // Detect framework for environment
  app.post(
    '/:environmentId/detect-framework',
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

      // Only support local repositories for framework detection
      if (environment.repository.type !== 'local') {
        throw new Error('Framework detection is only supported for local repositories')
      }

      // Re-detect framework and update environment
      const updatedEnvironment = await environmentService.detectAndUpdateFramework(environmentId)

      return updatedEnvironment.previewConfig
    },
  )
}
