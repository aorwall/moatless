import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { RepositoryNotFoundError } from '@moatless/shared/constants/errors'
import {
  CreateRepositorySchema,
  FileSearchSchema,
  UpdateRepositorySchema,
} from '@moatless/shared/schemas/repository.schemas'
import type { RepositoryList } from '@moatless/shared/types/repository'

export async function registerRepositoryRoutes(app: Hono<AppBindings>, prefix: string) {

  // Create repository
  app.post(prefix, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    
    const body = await c.req.json()
    const validatedBody = CreateRepositorySchema.parse(body)

    const repository = await repositoryService.createRepository(validatedBody)
    return c.json(repository)
  })

  // List repositories
  app.get(prefix, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    
    const repositories = await repositoryService.listRepositories()
    return c.json({ repositories } as RepositoryList)
  })

  // Get repository by ID
  app.get(`${prefix}/:repositoryId`, async (c) => {
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
  })

  // Update repository
  app.put(`${prefix}/:repositoryId`, async (c) => {
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
  })

  // Delete repository
  app.delete(`${prefix}/:repositoryId`, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    
    const repositoryId = c.req.param('repositoryId')
    if (!repositoryId) {
      throw new HTTPException(400, { message: 'Repository ID is required' })
    }
    
    await repositoryService.deleteRepository(repositoryId)
    return c.body(null, 204)
  })

  // Run setup script
  app.post(`${prefix}/:repositoryId/run-setup`, async (c) => {
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

    // For now, just return a mock success response
    // In a real implementation, this would use workspace service to run the setup commands
    return c.json({
      success: true,
      output: 'Setup completed successfully',
    })
  })

  // Search files in repository
  app.post(`${prefix}/:repositoryId/search-files`, async (c) => {
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
  })

  // Get git branches
  app.get(`${prefix}/:repositoryId/branches`, async (c) => {
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
  })

  // Detect framework
  app.post(`${prefix}/:repositoryId/detect-framework`, async (c) => {
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
  })
}
