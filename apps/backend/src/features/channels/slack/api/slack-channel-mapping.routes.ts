import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../../app'
import { appContainer } from '../../../../container/container'
import { getSessionFromContext } from '../../../../middleware/auth.middleware'
import { logger } from '../../../../utils/logger'
import { SlackChannelMappingService } from '../services/slack-channel-mapping.service'

/**
 * Register Slack channel mapping routes.
 * These routes manage channel-to-repository mappings.
 */
export function registerSlackChannelMappingRoutes(app: Hono<AppBindings>) {
  /**
   * POST /api/v1/channels/slack/mappings
   *
   * Create a new channel-to-repository mapping.
   * Only repository owners can create mappings.
   */
  app.post('/api/v1/channels/slack/mappings', async (c) => {
    const session = getSessionFromContext(c)
    if (!session) {
      throw new HTTPException(401, { message: 'Authentication required' })
    }

    const body = await c.req.json<{
      channelId: string
      teamId: string
      repositoryId: string
    }>()

    if (!(body.channelId && body.teamId && body.repositoryId)) {
      throw new HTTPException(400, { message: 'channelId, teamId, and repositoryId are required' })
    }

    const mappingService = appContainer.get(SlackChannelMappingService)

    // Check authorization
    const isAuthorized = await mappingService.isAuthorized(body.repositoryId, session.userId)
    if (!isAuthorized) {
      throw new HTTPException(403, {
        message: 'Only repository owners can create channel mappings',
      })
    }

    try {
      const mapping = await mappingService.setDefaultRepository(
        body.channelId,
        body.teamId,
        body.repositoryId,
        session.userId,
      )

      return c.json(
        {
          id: mapping.id,
          channelId: mapping.channelId,
          teamId: mapping.teamId,
          repositoryId: mapping.repositoryId,
          createdBy: mapping.createdBy,
          createdAt: mapping.createdAt.toISOString(),
          updatedAt: mapping.updatedAt.toISOString(),
        },
        201,
      )
    } catch (error) {
      logger.error({ err: error }, 'Error creating channel mapping')
      if (error instanceof Error && error.message.includes('Repository not found')) {
        throw new HTTPException(404, { message: error.message })
      }
      throw error
    }
  })

  /**
   * GET /api/v1/channels/slack/mappings
   *
   * List all channel-to-repository mappings.
   */
  app.get('/api/v1/channels/slack/mappings', async (c) => {
    const session = getSessionFromContext(c)
    if (!session) {
      throw new HTTPException(401, { message: 'Authentication required' })
    }

    const mappingService = appContainer.get(SlackChannelMappingService)
    const mappings = await mappingService.listMappings()

    return c.json(
      mappings.map(({ mapping, repository }) => ({
        id: mapping.id,
        channelId: mapping.channelId,
        teamId: mapping.teamId,
        repositoryId: mapping.repositoryId,
        repositoryName: repository?.name ?? null,
        createdBy: mapping.createdBy,
        createdAt: mapping.createdAt.toISOString(),
        updatedAt: mapping.updatedAt.toISOString(),
      })),
    )
  })

  /**
   * GET /api/v1/channels/slack/mappings/:channelId/:teamId
   *
   * Get a specific channel mapping.
   */
  app.get('/api/v1/channels/slack/mappings/:channelId/:teamId', async (c) => {
    const session = getSessionFromContext(c)
    if (!session) {
      throw new HTTPException(401, { message: 'Authentication required' })
    }

    const { channelId, teamId } = c.req.param()

    const mappingService = appContainer.get(SlackChannelMappingService)
    const result = await mappingService.getMapping(channelId, teamId)

    if (!result) {
      throw new HTTPException(404, { message: 'Channel mapping not found' })
    }

    const { mapping, repository } = result
    return c.json({
      id: mapping.id,
      channelId: mapping.channelId,
      teamId: mapping.teamId,
      repositoryId: mapping.repositoryId,
      repositoryName: repository?.name ?? null,
      createdBy: mapping.createdBy,
      createdAt: mapping.createdAt.toISOString(),
      updatedAt: mapping.updatedAt.toISOString(),
    })
  })

  /**
   * PUT /api/v1/channels/slack/mappings/:channelId/:teamId
   *
   * Update a channel mapping.
   * Only the current repository owner can update the mapping.
   */
  app.put('/api/v1/channels/slack/mappings/:channelId/:teamId', async (c) => {
    const session = getSessionFromContext(c)
    if (!session) {
      throw new HTTPException(401, { message: 'Authentication required' })
    }

    const { channelId, teamId } = c.req.param()
    const body = await c.req.json<{ repositoryId: string }>()

    if (!body.repositoryId) {
      throw new HTTPException(400, { message: 'repositoryId is required' })
    }

    const mappingService = appContainer.get(SlackChannelMappingService)

    // Check authorization for new repository
    const isAuthorized = await mappingService.isAuthorized(body.repositoryId, session.userId)
    if (!isAuthorized) {
      throw new HTTPException(403, {
        message: 'Only repository owners can update channel mappings',
      })
    }

    try {
      const mapping = await mappingService.setDefaultRepository(
        channelId,
        teamId,
        body.repositoryId,
        session.userId,
      )

      return c.json({
        id: mapping.id,
        channelId: mapping.channelId,
        teamId: mapping.teamId,
        repositoryId: mapping.repositoryId,
        createdBy: mapping.createdBy,
        createdAt: mapping.createdAt.toISOString(),
        updatedAt: mapping.updatedAt.toISOString(),
      })
    } catch (error) {
      logger.error({ err: error }, 'Error updating channel mapping')
      if (error instanceof Error && error.message.includes('Repository not found')) {
        throw new HTTPException(404, { message: error.message })
      }
      throw error
    }
  })

  /**
   * DELETE /api/v1/channels/slack/mappings/:channelId/:teamId
   *
   * Delete a channel mapping.
   * Only repository owners can delete mappings.
   */
  app.delete('/api/v1/channels/slack/mappings/:channelId/:teamId', async (c) => {
    const session = getSessionFromContext(c)
    if (!session) {
      throw new HTTPException(401, { message: 'Authentication required' })
    }

    const { channelId, teamId } = c.req.param()

    const mappingService = appContainer.get(SlackChannelMappingService)

    // Get the mapping to check authorization
    const existingMapping = await mappingService.getMapping(channelId, teamId)
    if (!existingMapping) {
      throw new HTTPException(404, { message: 'Channel mapping not found' })
    }

    // Check authorization
    const isAuthorized = await mappingService.isAuthorized(
      existingMapping.mapping.repositoryId,
      session.userId,
    )
    if (!isAuthorized) {
      throw new HTTPException(403, {
        message: 'Only repository owners can delete channel mappings',
      })
    }

    const deleted = await mappingService.removeDefaultRepository(channelId, teamId)
    if (!deleted) {
      throw new HTTPException(404, { message: 'Channel mapping not found' })
    }

    return c.json({ message: 'Channel mapping deleted successfully' })
  })
}
