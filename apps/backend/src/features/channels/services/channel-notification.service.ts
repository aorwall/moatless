import type { Event, TaskIterationEvent } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import { EmailOutboundService } from '../email/services/email-outbound.service'
import { SlackMessageService } from '../slack/services/slack-message.service'
import { ThreadTrackerService } from './thread-tracker.service'

/**
 * Service for sending notifications to external channels when task iterations complete.
 * Subscribes to taskIteration:completed events and dispatches to the appropriate provider.
 */
@injectable()
export class ChannelNotificationService {
  private unsubscribe?: (() => void) | undefined

  constructor(
    @inject(EventService) private eventService: EventService,
    @inject(EmailOutboundService) private emailOutbound: EmailOutboundService,
    @inject(SlackMessageService) private slackMessage: SlackMessageService,
    @inject(ThreadTrackerService) private threadTracker: ThreadTrackerService,
  ) {}

  /**
   * Start listening for task iteration completed events.
   * Called once during app initialization.
   */
  startEventSubscription(): void {
    if (this.unsubscribe) {
      logger.warn('ChannelNotificationService: Event subscription already started')
      return
    }

    this.unsubscribe = this.eventService.subscribeAll(async (event: Event) => {
      if (this.isIterationCompletedEvent(event)) {
        await this.handleIterationCompleted(event)
      }
    })

    logger.info('ChannelNotificationService: Started event subscription')
  }

  /**
   * Stop event subscription (for graceful shutdown).
   */
  stopEventSubscription(): void {
    if (this.unsubscribe) {
      this.unsubscribe()
      this.unsubscribe = undefined
      logger.info('ChannelNotificationService: Stopped event subscription')
    }
  }

  /**
   * Type guard for taskIteration:completed events.
   */
  private isIterationCompletedEvent(event: Event): event is TaskIterationEvent {
    return event.type === 'taskIteration' && event.subType === 'completed'
  }

  /**
   * Handle a completed task iteration by sending notification to the originating service.
   */
  private async handleIterationCompleted(event: TaskIterationEvent): Promise<void> {
    const { taskId, payload } = event

    try {
      // Single lookup - thread includes provider type
      const thread = await this.threadTracker.findAnyThreadByTaskId(taskId)
      if (!thread) {
        // Task not from a channel - nothing to do
        return
      }

      const message = this.formatResultMessage(payload)

      // Dispatch based on provider type
      switch (thread.provider) {
        case 'email':
          if (this.emailOutbound.isEnabled()) {
            await this.emailOutbound.sendTaskReply(taskId, message)
            logger.debug({ taskId, provider: 'email' }, 'Sent iteration notification')
          }
          break
        case 'slack':
          if (this.slackMessage.isEnabled()) {
            await this.slackMessage.sendTaskReply(taskId, message)
            logger.debug({ taskId, provider: 'slack' }, 'Sent iteration notification')
          }
          break
      }
    } catch (error) {
      logger.error({ err: error, taskId }, 'Failed to send iteration notification to channel')
    }
  }

  /**
   * Format the iteration result into a human-readable message.
   */
  private formatResultMessage(payload: TaskIterationEvent['payload']): string {
    const result = (payload as { result?: string }).result

    if (result) {
      // Truncate long results for external services
      const maxLength = 2000
      if (result.length > maxLength) {
        return `${result.slice(0, maxLength)}...\n\n_(Message truncated)_`
      }
      return result
    }

    return `Iteration ${payload.iterationNumber} completed.`
  }
}
