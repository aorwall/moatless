import type { Hono } from 'hono'
import type { AppBindings } from '../../../../app'
import { appContainer } from '../../../../container/container'
import { logger } from '../../../../utils/logger'
import { SlackEventService, type SlackEventWrapper } from '../services/slack-event.service'
import { shouldSkipSlackVerification, verifySlackSignature } from '../utils/slack-signature.utils'

/**
 * Register Slack event routes.
 * These routes handle events from the Slack Events API.
 */
export function registerSlackEventRoutes(app: Hono<AppBindings>) {
  /**
   * POST /api/v1/integrations/slack/events
   *
   * Receives events from Slack Events API.
   * Handles URL verification and app_mention events.
   */
  app.post('/api/v1/integrations/slack/events', async (c) => {
    const signingSecret = process.env['SLACK_SIGNING_SECRET']
    const rawBody = await c.req.text()

    // Verify signature if configured
    if (!shouldSkipSlackVerification(signingSecret)) {
      const signature = c.req.header('X-Slack-Signature')
      const timestamp = c.req.header('X-Slack-Request-Timestamp')

      if (!verifySlackSignature(signature, timestamp, rawBody, signingSecret!)) {
        logger.warn('Invalid Slack request signature')
        return c.json({ error: 'Invalid signature' }, 401)
      }
    }

    let eventWrapper: SlackEventWrapper
    try {
      eventWrapper = JSON.parse(rawBody) as SlackEventWrapper
    } catch (error) {
      logger.error({ err: error }, 'Failed to parse Slack event body')
      return c.json({ ok: true })
    }

    // Handle URL verification challenge (must be synchronous)
    if (eventWrapper.type === 'url_verification') {
      return c.json({ challenge: eventWrapper.challenge })
    }

    // Acknowledge immediately - process asynchronously
    const eventService = appContainer.get(SlackEventService)
    eventService.processAndReply(eventWrapper).catch((error) => {
      logger.error(
        { err: error, eventId: eventWrapper.event_id },
        'Async Slack event processing failed',
      )
    })

    return c.json({ ok: true })
  })
}
