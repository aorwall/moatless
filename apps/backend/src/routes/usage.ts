import type { OpenAPIHono } from '@hono/zod-openapi'
import { getSessionUsageRoute, getUsageRoute } from '@moatless/api-contract/usage'
import type {
  ClaudeSessionUsageResponse,
  ClaudeUsageResponse,
  RepositoryConfig,
} from '@moatless/types'
import { RepositoryNotFoundError } from '@moatless/types'
import type { AppBindings } from '../app'
import { appContainer } from '../container/container'
import { ClaudeService } from '../features/agents/index'
import { RepositoryService } from '../features/repositories/services/repository.service'
import { handleServiceError } from '../utils/error-handler'

export function registerUsageRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * GET /api/v1/usage
   * Get Claude API usage data with limits and statistics
   */
  app.openapi(getUsageRoute, async (c) => {
    try {
      const validatedQuery = c.req.valid('query')

      // Get repository context if repositoryId is provided
      let repository: RepositoryConfig | undefined
      if (validatedQuery.repositoryId) {
        const repositoryService = appContainer.get(RepositoryService)
        const foundRepository = await repositoryService.getRepositoryById(
          validatedQuery.repositoryId,
        )

        if (!foundRepository) {
          throw new RepositoryNotFoundError()
        }

        repository = foundRepository
      }

      const claudeService = appContainer.get(ClaudeService)
      const usageData: ClaudeUsageResponse = await claudeService.getUsage(
        validatedQuery.plan,
        repository,
      )

      return c.json(usageData)
    } catch (error) {
      handleServiceError(error, 'Get Claude usage data')
    }
  })

  /**
   * GET /api/v1/usage/session/{sessionId}
   * Get usage data for a specific session
   */
  app.openapi(getSessionUsageRoute, async (c) => {
    try {
      const params = c.req.valid('param')
      const sessionId = params.sessionId as string

      // Note: repositoryId parameter is no longer used since we search for session files globally

      const claudeService = appContainer.get(ClaudeService)
      const sessionUsage: ClaudeSessionUsageResponse =
        await claudeService.getSessionUsage(sessionId)

      // Check if session was found (has entries)
      if (sessionUsage.messageCount === 0 && !sessionUsage.startTime) {
        throw new Error(`Session not found: ${sessionId}`)
      }

      return c.json(sessionUsage)
    } catch (error) {
      handleServiceError(error, 'Get session usage data')
    }
  })
}
