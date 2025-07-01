import type { FastifyInstance } from 'fastify'
import type { z } from 'zod'
import { RepositoryNotFoundError } from '../../shared/constants/errors.js'
import {
  CreateRepositorySchema,
  FileSearchSchema,
  RepositoryIdParamsSchema,
  UpdateRepositorySchema,
} from '../../shared/schemas/repository.schemas.js'
import type { RepositoryList } from '../../shared/types/repository.js'

export async function registerRepositoryRoutes(app: FastifyInstance) {
  const { repository: repositoryService } = app.services

  // Create repository
  app.post(
    '/',
    {
      schema: {
        body: CreateRepositorySchema,
      },
    },
    async (request, _reply) => {
      const body = request.body as z.infer<typeof CreateRepositorySchema>

      const repository = await repositoryService.createRepository(body)
      return repository
    },
  )

  // List repositories
  app.get('/', async (_request, _reply) => {
    const repositories = await repositoryService.listRepositories()
    return { repositories } as RepositoryList
  })

  // Get repository by ID
  app.get(
    '/:repositoryId',
    {
      schema: {
        params: RepositoryIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const params = request.params as z.infer<typeof RepositoryIdParamsSchema>
      const repository = await repositoryService.getRepositoryById(params.repositoryId)

      if (!repository) {
        throw new RepositoryNotFoundError()
      }

      return repository
    },
  )

  // Update repository
  app.put(
    '/:repositoryId',
    {
      schema: {
        params: RepositoryIdParamsSchema,
        body: UpdateRepositorySchema,
      },
    },
    async (request, _reply) => {
      const params = request.params as z.infer<typeof RepositoryIdParamsSchema>
      const body = request.body as z.infer<typeof UpdateRepositorySchema>

      const repository = await repositoryService.updateRepository(params.repositoryId, body)
      return repository
    },
  )

  // Delete repository
  app.delete(
    '/:repositoryId',
    {
      schema: {
        params: RepositoryIdParamsSchema,
      },
    },
    async (request, reply) => {
      const params = request.params as z.infer<typeof RepositoryIdParamsSchema>
      await repositoryService.deleteRepository(params.repositoryId)
      reply.code(204)
    },
  )

  // Run setup script
  app.post(
    '/:repositoryId/run-setup',
    {
      schema: {
        params: RepositoryIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const params = request.params as z.infer<typeof RepositoryIdParamsSchema>
      const repository = await repositoryService.getRepositoryById(params.repositoryId)

      if (!repository) {
        throw new RepositoryNotFoundError()
      }

      // For now, just return a mock success response
      // In a real implementation, this would use workspace service to run the setup commands
      return {
        success: true,
        output: 'Setup completed successfully',
      }
    },
  )

  // Search files in repository
  app.post(
    '/:repositoryId/search-files',
    {
      schema: {
        params: RepositoryIdParamsSchema,
        body: FileSearchSchema,
      },
    },
    async (request, _reply) => {
      const params = request.params as z.infer<typeof RepositoryIdParamsSchema>
      const body = request.body as z.infer<typeof FileSearchSchema>

      const repository = await repositoryService.getRepositoryById(params.repositoryId)
      if (!repository) {
        throw new RepositoryNotFoundError()
      }

      if (repository.repository.type !== 'local') {
        throw new Error('File search is only supported for local repositories')
      }

      const results = await repositoryService.searchFiles(params.repositoryId, body)
      return results
    },
  )

  // Get git branches
  app.get(
    '/:repositoryId/branches',
    {
      schema: {
        params: RepositoryIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const params = request.params as z.infer<typeof RepositoryIdParamsSchema>
      const repository = await repositoryService.getRepositoryById(params.repositoryId)

      if (!repository) {
        throw new RepositoryNotFoundError()
      }

      if (repository.repository.type !== 'local') {
        throw new Error('Branch listing is only supported for local repositories')
      }

      const branches = await repositoryService.getBranches(repository.repository.path)
      return { branches }
    },
  )
}
