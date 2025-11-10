import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  deleteClaudeConfigRoute,
  getClaudeConfigRoute,
  getClaudeOAuthUrlRoute,
  getClaudeSettingsRoute,
  saveClaudePATRoute,
  updateClaudeSettingsRoute,
} from '@moatless/api-contract/settings'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { handleServiceError } from '../../../utils/error-handler'
import { logger } from '../../../utils/logger'
import { SessionContext } from '../../auth/services/session-context'
import { ClaudeSettingsService } from '../services/claude-settings.service'
import { ClaudeTokenService } from '../services/claude-token.service'

export function registerClaudeSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/settings/claude
  app.openapi(getClaudeSettingsRoute, async (c) => {
    try {
      const claudeSettingsService = appContainer.get(ClaudeSettingsService)
      const settings = await claudeSettingsService.getClaudeSettingsResponse()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get Claude settings')
    }
  })

  // PUT /api/v1/settings/claude
  app.openapi(updateClaudeSettingsRoute, async (c) => {
    try {
      const claudeSettingsService = appContainer.get(ClaudeSettingsService)
      const update = c.req.valid('json')
      const settings = await claudeSettingsService.updateClaudeSettingsSection(update)
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Update Claude settings')
    }
  })

  // GET /api/v1/settings/claude/config - Get Claude connection status
  app.openapi(getClaudeConfigRoute, async (c) => {
    try {
      const claudeTokenService = appContainer.get(ClaudeTokenService)
      const config = await claudeTokenService.getConnectionConfig()
      return c.json(config)
    } catch (error) {
      logger.error('Failed to get Claude configuration', error)
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, { message: 'Failed to retrieve configuration' })
    }
  })

  // GET /api/v1/settings/claude/oauth/url - Generate OAuth URL
  app.openapi(getClaudeOAuthUrlRoute, async (c) => {
    try {
      const claudeTokenService = appContainer.get(ClaudeTokenService)

      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      const result = await claudeTokenService.generateServiceOAuthUrl(userId)
      return c.json(result)
    } catch (error) {
      logger.error('Failed to generate Claude OAuth URL', error)
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error) {
        throw new HTTPException(500, { message: error.message })
      }
      throw new HTTPException(500, { message: 'Failed to generate OAuth URL' })
    }
  })

  // DELETE /api/v1/settings/claude/config - Disconnect Claude account
  app.openapi(deleteClaudeConfigRoute, async (c) => {
    try {
      const claudeTokenService = appContainer.get(ClaudeTokenService)

      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      await claudeTokenService.disconnect(userId)

      return c.json({
        success: true,
        message: 'Claude account disconnected successfully',
      })
    } catch (error) {
      logger.error('Failed to disconnect Claude', error)
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, { message: 'Failed to disconnect Claude' })
    }
  })

  // PUT /api/v1/settings/claude/pat - Save Personal Access Token
  app.openapi(saveClaudePATRoute, async (c) => {
    try {
      const claudeTokenService = appContainer.get(ClaudeTokenService)

      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      const { accessToken } = c.req.valid('json')
      await claudeTokenService.savePersonalAccessToken(userId, accessToken)

      return c.json({
        success: true,
        message: 'Claude Personal Access Token saved successfully',
      })
    } catch (error) {
      logger.error('Failed to save Claude Personal Access Token', error)
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
