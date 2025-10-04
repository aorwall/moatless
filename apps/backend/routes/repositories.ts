import type { RepositoryList } from '@moatless/types'
import {
  CommandExecutionOptionsSchema,
  CreateRepositorySchema,
  FileSearchSchema,
  RepositoryNotFoundError,
  UpdateRepositorySchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'

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

    const includeVibeRepos = c.req.query('includeVibeRepos') === 'true'
    const repositories = await repositoryService.listRepositories(includeVibeRepos)
    return c.json({ repositories } as RepositoryList)
  })

  // Get suggested repositories from Claude projects
  // IMPORTANT: This must be registered before dynamic routes like /:repositoryId
  app.get(`${prefix}/suggestions`, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services

    const onlyGitRepos = c.req.query('onlyGitRepos') !== 'false' // Default to true
    const suggestions = await repositoryService.getSuggestedRepositories(onlyGitRepos)

    return c.json({ suggestions })
  })

  // Get single repository with all configuration
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

  // Execute commands in repository with streaming output
  app.post(`${prefix}/:repositoryId/execute-commands`, async (c) => {
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
  app.post(`${prefix}/:repositoryId/verify`, async (c) => {
    const services = c.get('services')
    const { repository: repositoryService } = services

    const repositoryId = c.req.param('repositoryId')
    if (!repositoryId) {
      throw new HTTPException(400, { message: 'Repository ID is required' })
    }

    const verificationResult = await repositoryService.verifyRepositorySetup(repositoryId)
    return c.json(verificationResult)
  })
}
