import { logger } from '../../../utils/logger'

/**
 * Configuration for email channel integration.
 */
export interface EmailChannelConfig {
  enabled: boolean
  sendgrid: {
    apiKey?: string | undefined
    webhookSecret?: string | undefined
  }
  fromAddress: string
  fromName: string
  inboxDomain: string
}

/**
 * Configuration for Slack channel integration.
 */
export interface SlackChannelConfig {
  enabled: boolean
  botToken?: string | undefined
  signingSecret?: string | undefined
}

/**
 * Complete channel configuration.
 */
export interface ChannelsConfig {
  email: EmailChannelConfig
  slack: SlackChannelConfig
}

// Singleton instance
let channelsConfig: ChannelsConfig | null = null

/**
 * Check if a feature flag is enabled.
 * Accepts 'true' or '1' as truthy values.
 */
function isFeatureFlagEnabled(envVar: string): boolean {
  const value = process.env[envVar]
  return value === 'true' || value === '1'
}

/**
 * Load channel configuration from environment variables.
 * This does NOT validate - call validateChannelsConfig() separately.
 */
export function loadChannelsConfig(): ChannelsConfig {
  const emailEnabled = isFeatureFlagEnabled('FF_EMAIL_CHANNEL')
  const slackEnabled = isFeatureFlagEnabled('FF_SLACK_CHANNEL')

  const config: ChannelsConfig = {
    email: {
      enabled: emailEnabled,
      sendgrid: {
        apiKey: process.env['SENDGRID_API_KEY'],
        webhookSecret: process.env['SENDGRID_WEBHOOK_SECRET'],
      },
      // New EMAIL_* vars with fallback to old SENDGRID_* vars for backward compatibility
      fromAddress:
        process.env['EMAIL_FROM_ADDRESS'] ||
        process.env['SENDGRID_FROM_EMAIL'] ||
        'noreply@inbox.moatless.ai',
      fromName: process.env['EMAIL_FROM_NAME'] || process.env['SENDGRID_FROM_NAME'] || 'Moatless',
      inboxDomain: process.env['EMAIL_INBOX_DOMAIN'] || 'inbox.moatless.ai',
    },
    slack: {
      enabled: slackEnabled,
      botToken: process.env['SLACK_BOT_TOKEN'],
      signingSecret: process.env['SLACK_SIGNING_SECRET'],
    },
  }

  // Cache the config
  channelsConfig = config

  return config
}

/**
 * Validate channel configuration.
 * Throws an error if a channel is enabled but required configuration is missing.
 *
 * @param config - The channel configuration to validate
 * @throws Error if validation fails
 */
export function validateChannelsConfig(config: ChannelsConfig): void {
  const errors: string[] = []

  // Validate email channel
  if (config.email.enabled) {
    if (!config.email.sendgrid.apiKey) {
      errors.push('FF_EMAIL_CHANNEL is enabled but SENDGRID_API_KEY is not set')
    }
    logger.info(
      { inboxDomain: config.email.inboxDomain, fromAddress: config.email.fromAddress },
      'Email channel enabled',
    )
  }

  // Validate Slack channel
  if (config.slack.enabled) {
    if (!config.slack.botToken) {
      errors.push('FF_SLACK_CHANNEL is enabled but SLACK_BOT_TOKEN is not set')
    }
    logger.info('Slack channel enabled')
  }

  if (errors.length > 0) {
    const errorMessage = `Channel configuration validation failed:\n  - ${errors.join('\n  - ')}`
    logger.error(errorMessage)
    throw new Error(errorMessage)
  }
}

/**
 * Get the cached channel configuration.
 * Must call loadChannelsConfig() first during app initialization.
 *
 * @returns The channel configuration
 * @throws Error if configuration hasn't been loaded
 */
export function getChannelsConfig(): ChannelsConfig {
  if (!channelsConfig) {
    throw new Error('Channel configuration not loaded. Call loadChannelsConfig() first.')
  }
  return channelsConfig
}

/**
 * Check if email channel is enabled.
 * Convenience function for quick checks.
 */
export function isEmailChannelEnabled(): boolean {
  return getChannelsConfig().email.enabled
}

/**
 * Check if Slack channel is enabled.
 * Convenience function for quick checks.
 */
export function isSlackChannelEnabled(): boolean {
  return getChannelsConfig().slack.enabled
}
