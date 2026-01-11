import { inject, injectable } from 'inversify'
import { logger } from '../../../../utils/logger'
import type { SlackThreadProperties } from '../../db/channel-thread.schema'
import { ThreadTrackerService } from '../../services/thread-tracker.service'

/**
 * Service for sending messages to Slack.
 * Handles posting replies to threads when agents complete tasks.
 */
@injectable()
export class SlackMessageService {
  private botToken: string | null = null

  constructor(@inject(ThreadTrackerService) private threadTracker: ThreadTrackerService) {
    this.initializeConfig()
  }

  /**
   * Initialize Slack configuration from environment.
   */
  private initializeConfig(): void {
    this.botToken = process.env['SLACK_BOT_TOKEN'] || null
    if (!this.botToken) {
      logger.warn('SLACK_BOT_TOKEN not configured - Slack messaging disabled')
    } else {
      logger.info('Slack message service initialized')
    }
  }

  /**
   * Check if Slack messaging is enabled.
   */
  isEnabled(): boolean {
    return this.botToken !== null
  }

  /**
   * Send a reply message to a Slack thread for a task.
   * Called when an agent completes an iteration.
   *
   * @param taskId - The task ID
   * @param message - The message content to send
   */
  async sendTaskReply(taskId: string, message: string): Promise<void> {
    if (!this.botToken) {
      logger.debug({ taskId }, 'Slack messaging disabled - skipping reply')
      return
    }

    // Find Slack thread for this task
    const thread = await this.threadTracker.findThreadByTaskId(taskId, 'slack')
    if (!thread) {
      logger.debug({ taskId }, 'No Slack thread found for task - skipping reply')
      return
    }

    const properties = thread.properties as SlackThreadProperties

    try {
      await this.postMessage({
        channel: properties.channelId,
        threadTs: thread.externalThreadId,
        text: message,
      })

      // Update thread timestamp
      if (thread.id) {
        await this.threadTracker.updateLastMessageAt(thread.id)
      }

      logger.info({ taskId, channel: properties.channelId }, 'Slack reply sent')
    } catch (error) {
      logger.error({ err: error, taskId }, 'Failed to send Slack reply')
    }
  }

  /**
   * Post a message to a Slack channel/thread.
   */
  async postMessage(params: { channel: string; threadTs?: string; text: string }): Promise<void> {
    if (!this.botToken) {
      throw new Error('Slack messaging not configured')
    }

    const payload: Record<string, unknown> = {
      channel: params.channel,
      text: params.text,
    }

    if (params.threadTs) {
      payload['thread_ts'] = params.threadTs
    }

    const response = await fetch('https://slack.com/api/chat.postMessage', {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${this.botToken}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(payload),
    })

    const data = (await response.json()) as { ok: boolean; error?: string }

    if (!data.ok) {
      throw new Error(`Slack API error: ${data.error}`)
    }
  }

  /**
   * Post an error message to a Slack channel/thread.
   */
  async postError(
    channel: string,
    threadTs: string | undefined,
    errorMessage: string,
  ): Promise<void> {
    await this.postMessage({
      channel,
      ...(threadTs ? { threadTs } : {}),
      text: `:warning: ${errorMessage}`,
    })
  }
}
