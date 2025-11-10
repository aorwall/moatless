import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  deleteGitLabConfigRoute,
  getGitLabConfigRoute,
  saveGitLabPATRoute,
} from '@moatless/api-contract/settings'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { logger } from '../../../utils/logger'
import { GitLabTokenService } from '../services/gitlab-token.service'
import { SessionContext } from '../services/session-context'

/**
 * Register GitLab settings routes in the auth feature.
 * These routes manage GitLab token connections (PAT only).
 */
export function registerGitLabSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/settings/gitlab/config
  app.openapi(getGitLabConfigRoute, async (c) => {
    try {
      const gitlabTokenService = appContainer.get(GitLabTokenService)

      const config = await gitlabTokenService.getConnectionConfig()
      return c.json(config)
    } catch (error) {
      logger.error('Failed to get GitLab configuration', error)
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, { message: 'Failed to retrieve configuration' })
    }
  })

  // DELETE /api/v1/settings/gitlab/config
  app.openapi(deleteGitLabConfigRoute, async (c) => {
    try {
      const gitlabTokenService = appContainer.get(GitLabTokenService)

      // Get current user from session
      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      // Delete user's GitLab provider token
      await gitlabTokenService.deleteProviderToken(userId)

      return c.json({
        success: true,
        message: 'GitLab account disconnected successfully',
      })
    } catch (error) {
      logger.error('Failed to disconnect GitLab', error)
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, { message: 'Failed to disconnect GitLab' })
    }
  })

  // PUT /api/v1/settings/gitlab/pat
  app.openapi(saveGitLabPATRoute, async (c) => {
    try {
      const gitlabTokenService = appContainer.get(GitLabTokenService)

      // Get current user from session
      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      const { accessToken } = c.req.valid('json')

      // Save Personal Access Token
      await gitlabTokenService.saveProviderPAT(userId, accessToken)

      return c.json({
        success: true,
        message: 'GitLab Personal Access Token saved successfully',
      })
    } catch (error) {
      logger.error('Failed to save GitLab Personal Access Token', error)
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error) {
        throw new HTTPException(400, { message: error.message })
      }
      throw new HTTPException(500, { message: 'Failed to save Personal Access Token' })
    }
  })
}
