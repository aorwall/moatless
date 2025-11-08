import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  deleteGitHubConfigRoute,
  exchangeGitHubTokenRoute,
  getGitHubConfigRoute,
  getGitHubOAuthUrlRoute,
  saveGitHubPATRoute,
} from '@moatless/api-contract/settings'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { logger } from '../../../utils/logger'
import { GitHubTokenService } from '../services/github-token.service'
import { SessionContext } from '../services/session-context'

/**
 * Register GitHub settings routes in the auth feature.
 * These routes manage GitHub token connections (PAT and service OAuth).
 */
export function registerGitHubSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/settings/github/config
  app.openapi(getGitHubConfigRoute, async (c) => {
    try {
      const githubTokenService = appContainer.get(GitHubTokenService)

      const config = await githubTokenService.getConnectionConfig()
      return c.json(config)
    } catch (error) {
      logger.error('Failed to get GitHub configuration', error)
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, { message: 'Failed to retrieve configuration' })
    }
  })

  // GET /api/v1/settings/github/oauth/url
  app.openapi(getGitHubOAuthUrlRoute, async (c) => {
    try {
      const githubTokenService = appContainer.get(GitHubTokenService)

      // Get current user from session
      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      // Generate OAuth URL
      const result = await githubTokenService.generateServiceOAuthUrl(userId)
      return c.json(result)
    } catch (error) {
      logger.error('Failed to generate GitHub OAuth URL', error)
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error) {
        throw new HTTPException(500, { message: error.message })
      }
      throw new HTTPException(500, { message: 'Failed to generate OAuth URL' })
    }
  })

  // POST /api/v1/settings/github/oauth/token
  app.openapi(exchangeGitHubTokenRoute, async (c) => {
    try {
      const githubTokenService = appContainer.get(GitHubTokenService)

      // Get current user from session
      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      const { code, state } = c.req.valid('json')

      // Exchange code for token
      await githubTokenService.exchangeServiceOAuthToken(code, state, userId)

      return c.json({
        success: true,
        message: 'GitHub account connected successfully',
      })
    } catch (error) {
      logger.error('Failed to exchange GitHub OAuth token', error)
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error) {
        throw new HTTPException(400, { message: error.message })
      }
      throw new HTTPException(500, { message: 'Failed to exchange OAuth token' })
    }
  })

  // DELETE /api/v1/settings/github/config
  app.openapi(deleteGitHubConfigRoute, async (c) => {
    try {
      const githubTokenService = appContainer.get(GitHubTokenService)

      // Get current user from session
      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      // Delete user's GitHub provider token
      await githubTokenService.deleteProviderToken(userId)

      return c.json({
        success: true,
        message: 'GitHub account disconnected successfully',
      })
    } catch (error) {
      logger.error('Failed to disconnect GitHub', error)
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, { message: 'Failed to disconnect GitHub' })
    }
  })

  // PUT /api/v1/settings/github/pat
  app.openapi(saveGitHubPATRoute, async (c) => {
    try {
      const githubTokenService = appContainer.get(GitHubTokenService)

      // Get current user from session
      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      const { accessToken } = c.req.valid('json')

      // Save Personal Access Token
      await githubTokenService.saveProviderPAT(userId, accessToken)

      return c.json({
        success: true,
        message: 'GitHub Personal Access Token saved successfully',
      })
    } catch (error) {
      logger.error('Failed to save GitHub Personal Access Token', error)
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
