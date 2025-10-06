import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  createRepositoryRoute,
  deleteRepositoryRoute,
  getRepositoryBranchesRoute,
  getRepositoryRoute,
  getRepositorySuggestionsRoute,
  listRepositoriesRoute,
  searchRepositoryFilesRoute,
  updateRepositoryRoute,
  verifyRepositoryRoute,
} from '@moatless/api-contract/repositories'
import type { RepositoryList } from '@moatless/types'
import { CommandExecutionOptionsSchema, RepositoryNotFoundError } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'

export function registerRepositoryRoutes(app: OpenAPIHono<AppBindings>) {
  // Create repository
  app.openapi(createRepositoryRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const validatedBody = c.req.valid('json')

    const repository = await repositoryService.createRepository(validatedBody)
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
  // IMPORTANT: This must be registered before dynamic routes like /:repositoryId
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

  // Search files in repository
  app.openapi(searchRepositoryFilesRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const { repositoryId } = c.req.valid('param')
    const validatedBody = c.req.valid('json')

    const repository = await repositoryService.getRepositoryById(repositoryId)
    if (!repository) {
      throw new RepositoryNotFoundError()
    }

    const results = await repositoryService.searchFiles(repositoryId, {
      query: validatedBody.query,
      ...(validatedBody.limit !== undefined && { limit: validatedBody.limit }),
    })
    return c.json(results)
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

  // Execute commands in repository with streaming output
  // Note: SSE endpoints don't integrate well with OpenAPI spec, using plain Hono
  app.post('/api/v1/repositories/:repositoryId/execute-commands', async (c) => {
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
  app.openapi(verifyRepositoryRoute, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services
    const { repositoryId } = c.req.valid('param')

    const verificationResult = await repositoryService.verifyRepositorySetup(repositoryId)
    return c.json(verificationResult)
  })
}
