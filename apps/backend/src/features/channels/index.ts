import type { Hono } from 'hono'
import type { AppBindings } from '../../app'
import { registerEmailWebhookRoutes } from './email/api/email-webhook.routes'
import { registerSlackEventRoutes } from './slack/api/slack-events.routes'

export * from './db/channel-thread.repository'
// Database schemas and repositories
export * from './db/channel-thread.schema'
// Email channel
export * from './email/services/email-inbound.service'
export * from './email/services/email-outbound.service'
// Shared services
export * from './services/channel-notification.service'
export * from './services/thread-tracker.service'
export * from './services/user-resolver.service'

// Slack channel
export * from './slack/services/slack-event.service'
export * from './slack/services/slack-message.service'

// Utilities
export * from './utils/short-id.utils'

/**
 * Register all channel routes.
 */
export function registerChannelRoutes(app: Hono<AppBindings>): void {
  registerEmailWebhookRoutes(app)
  registerSlackEventRoutes(app)
}
