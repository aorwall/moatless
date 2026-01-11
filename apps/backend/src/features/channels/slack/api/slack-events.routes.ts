import type { Hono } from 'hono'
import type { AppBindings } from '../../../../app'
import { appContainer } from '../../../../container/container'
import { logger } from '../../../../utils/logger'
import { SlackEventService, type SlackEventWrapper } from '../services/slack-event.service'
import { SlackMessageService } from '../services/slack-message.service'
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

    try {
      const eventWrapper = JSON.parse(rawBody) as SlackEventWrapper

      // Handle URL verification challenge
      if (eventWrapper.type === 'url_verification') {
        return c.json({ challenge: eventWrapper.challenge })
      }

      // Process the event
      const eventService = appContainer.get(SlackEventService)
      const result = await eventService.processEvent(eventWrapper)

      // Send reply if needed
      if (result.shouldReply && result.replyMessage) {
        const messageService = appContainer.get(SlackMessageService)
        const event = eventWrapper.event

        if (messageService.isEnabled()) {
          await messageService.postMessage({
            channel: event.channel,
            threadTs: event.thread_ts || event.ts,
            text: result.replyMessage,
          })
        }
      }

      // Return 200 to acknowledge receipt
      return c.json({ ok: true })
    } catch (error) {
      logger.error({ err: error }, 'Error processing Slack event')
      // Return 200 to prevent Slack from retrying
      return c.json({ ok: true, error: 'Internal error' })
    }
  })
}
