import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  createRepositoryRoute,
  deleteRepositoryRoute,
  getRepositoryBranchesRoute,
  getRepositoryRoute,
  getRepositorySuggestionsRoute,
  listRepositoriesRoute,
  updateRepositoryRoute,
} from '@moatless/api-contract/repositories'
import type { RepositoryList } from '@moatless/types'
import { RepositoryNotFoundError } from '@moatless/types'
import type { AppBindings } from '../app.js'
import { getGitHubTokenFromContext } from '../shared/http/middleware/auth.middleware.js'

export function registerRepositoryRoutes(app: OpenAPIHono<AppBindings>) {
  // Create repository
  app.openapi(createRepositoryRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const validatedBody = c.req.valid('json')

    // Get GitHub OAuth token from session if available
    const sessionToken = getGitHubTokenFromContext(c)

    const repository = await repositoryService.createRepository(validatedBody, sessionToken)
    return c.json(repository)
  })

  // List repositories
  app.openapi(listRepositoriesRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services

    const { includeVibeRepos } = c.req.valid('query')
    const repositories = await repositoryService.listRepositories(includeVibeRepos === 'true')
    return c.json({ repositories } as RepositoryList)
  })

  // Get suggested repositories from Claude projects
  app.openapi(getRepositorySuggestionsRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services

    const { onlyGitRepos } = c.req.valid('query')
    const suggestions = await repositoryService.getSuggestedRepositories(onlyGitRepos !== 'false')

    return c.json({ suggestions })
  })

  // Get single repository with all configuration
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
  app.openapi(updateRepositoryRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const { repositoryId } = c.req.valid('param')
    const validatedBody = c.req.valid('json')

    const repository = await repositoryService.updateRepository(repositoryId, validatedBody)
    return c.json(repository)
  })

  // Delete repository
  app.openapi(deleteRepositoryRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const { repositoryId } = c.req.valid('param')

    await repositoryService.deleteRepository(repositoryId)
    return c.body(null, 204)
  })

  // Get git branches
  app.openapi(getRepositoryBranchesRoute, async (c) => {
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
}
