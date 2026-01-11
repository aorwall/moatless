import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  convertToTemplateRoute,
  createRepositoryRoute,
  deleteRepositoryRoute,
  getRepositoryBranchesRoute,
  getRepositoryRoute,
  listRepositoriesRoute,
  updateRepositoryRoute,
} from '@moatless/api-contract/repositories'
import type { RepositoryConfig, RepositoryList } from '@moatless/types'
import { RepositoryNotFoundError } from '@moatless/types'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { getGitHubTokenFromContext } from '../../../middleware/auth.middleware'
import { getChannelsConfig } from '../../channels/config/channels.config'
import { generateRepositoryEmailAddress } from '../../channels/email/utils/email-address.utils'
import { RepositoryService } from '../services/repository.service'

/**
 * Enrich repository response with channel information.
 * Only adds email info for remote repositories (not templates).
 *
 * @param repository - The repository to enrich
 * @returns Repository with channel info added if applicable
 */
function enrichWithChannelInfo(repository: RepositoryConfig): RepositoryConfig {
  // Only add channel info for remote repos, not templates
  if (repository.kind === 'template') {
    return repository
  }

  // Check if email channel is enabled
  let channelsConfig: ReturnType<typeof getChannelsConfig> | null = null
  try {
    channelsConfig = getChannelsConfig()
  } catch {
    // Channel config not loaded - channels not enabled
    return repository
  }

  if (!channelsConfig.email.enabled) {
    return repository
  }

  // Generate the email address for this repository
  const emailAddress = generateRepositoryEmailAddress(
    repository.name,
    channelsConfig.email.inboxDomain,
  )

  return {
    ...repository,
    channels: {
      email: {
        address: emailAddress,
      },
    },
  }
}

/**
 * Register repository routes
 */
export function registerRepositoryRoutes(app: OpenAPIHono<AppBindings>) {
  // Create repository
  app.openapi(createRepositoryRoute, async (c) => {
    const repositoryService = appContainer.get(RepositoryService)
    const validatedBody = c.req.valid('json')

    // Get GitHub OAuth token from session if available
    const sessionToken = getGitHubTokenFromContext(c)

    const repository = await repositoryService.createRepository(validatedBody, sessionToken)
    return c.json(repository)
  })

  // List repositories
  app.openapi(listRepositoriesRoute, async (c) => {
    const repositoryService = appContainer.get(RepositoryService)
    const repositories = await repositoryService.listRepositories()
    return c.json({ repositories } as RepositoryList)
  })

  // Get single repository with all configuration
  app.openapi(getRepositoryRoute, async (c) => {
    const repositoryService = appContainer.get(RepositoryService)
    const { repositoryId } = c.req.valid('param')

    const repository = await repositoryService.getRepositoryById(repositoryId)
    if (!repository) {
      throw new RepositoryNotFoundError()
    }

    // Enrich with channel information (email address for local/remote repos)
    const enriched = enrichWithChannelInfo(repository)

    return c.json(enriched)
  })

  // Update repository
  app.openapi(updateRepositoryRoute, async (c) => {
    const repositoryService = appContainer.get(RepositoryService)
    const { repositoryId } = c.req.valid('param')
    const validatedBody = c.req.valid('json')

    const repository = await repositoryService.updateRepository(repositoryId, validatedBody)
    return c.json(repository)
  })

  // Delete repository
  app.openapi(deleteRepositoryRoute, async (c) => {
    const repositoryService = appContainer.get(RepositoryService)
    const { repositoryId } = c.req.valid('param')

    await repositoryService.deleteRepository(repositoryId)
    return c.body(null, 204)
  })

  // Convert remote repository to template
  app.openapi(convertToTemplateRoute, async (c) => {
    const repositoryService = appContainer.get(RepositoryService)
    const { repositoryId } = c.req.valid('param')

    const template = await repositoryService.convertToTemplate(repositoryId)
    return c.json(template)
  })

  // Get git branches
  app.openapi(getRepositoryBranchesRoute, async (c) => {
    const repositoryService = appContainer.get(RepositoryService)
    const { repositoryId } = c.req.valid('param')

    const branches = await repositoryService.getBranches(repositoryId)
    return c.json({ branches })
  })
}
