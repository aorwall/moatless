import type { AgentEvent, Event, PendingResponsePayload, TaskIterationEvent } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import type { SlackThreadProperties } from '../db/channel-thread.schema'
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
   * Start listening for task iteration completed events and pending_response events.
   * Called once during app initialization.
   */
  startEventSubscription(): void {
    if (this.unsubscribe) {
      logger.warn('ChannelNotificationService: Event subscription already started')
      return
    }

    this.unsubscribe = this.eventService.subscribeAll(async (event: Event) => {
      if (this.isIterationFinishedEvent(event)) {
        await this.handleIterationFinished(event)
      } else if (this.isPendingResponseEvent(event)) {
        await this.handlePendingResponse(event as AgentEvent)
      }
    })

    logger.info('ChannelNotificationService: Started event subscription')
  }

  /**
   * Type guard for agent pending_response events.
   */
  private isPendingResponseEvent(event: Event): event is AgentEvent {
    return event.type === 'agent' && event.subType === 'pending_response'
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
   * Type guard for taskIteration completed or error events.
   */
  private isIterationFinishedEvent(event: Event): event is TaskIterationEvent {
    return (
      event.type === 'taskIteration' && (event.subType === 'completed' || event.subType === 'error')
    )
  }

  /**
   * Handle a finished task iteration by sending notification and swapping reaction.
   */
  private async handleIterationFinished(event: TaskIterationEvent): Promise<void> {
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

            // Swap reaction emoji on the triggering message
            const props = thread.properties as SlackThreadProperties
            if (props.messageTs) {
              const processingEmoji = process.env['SLACK_REACTION_PROCESSING'] || 'eyes'
              const doneEmoji =
                event.subType === 'completed'
                  ? process.env['SLACK_REACTION_DONE'] || 'white_check_mark'
                  : process.env['SLACK_REACTION_FAILED'] || 'x'

              await this.slackMessage.removeReaction(
                props.channelId,
                props.messageTs,
                processingEmoji,
              )
              await this.slackMessage.addReaction(props.channelId, props.messageTs, doneEmoji)
            }

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
      const maxLength = 4000
      if (result.length > maxLength) {
        return `${result.slice(0, maxLength)}...\n\n_(Message truncated)_`
      }
      return result
    }

    return `Iteration ${payload.iterationNumber} completed.`
  }

  /**
   * Handle pending_response event by sending notification to the appropriate channel.
   * Notifies users when the agent is waiting for their input (AskUserQuestion or ExitPlanMode).
   */
  private async handlePendingResponse(event: AgentEvent): Promise<void> {
    const { taskId, payload } = event
    const pendingPayload = payload as PendingResponsePayload

    try {
      // Look up channel thread by taskId
      const thread = await this.threadTracker.findAnyThreadByTaskId(taskId)
      if (!thread) {
        // Task not from a channel - nothing to do
        return
      }

      const taskUrl = this.getTaskUrl(taskId)
      const message = this.formatPendingResponseMessage(pendingPayload, taskUrl)

      // Dispatch based on provider type
      switch (thread.provider) {
        case 'email':
          if (this.emailOutbound.isEnabled()) {
            await this.emailOutbound.sendTaskReply(taskId, message)
            logger.debug(
              { taskId, provider: 'email', toolName: pendingPayload.toolName },
              'Sent pending response notification',
            )
          }
          break
        case 'slack':
          if (this.slackMessage.isEnabled()) {
            await this.slackMessage.sendTaskReply(taskId, message)

            // Add question mark reaction to indicate waiting for input
            const props = thread.properties as SlackThreadProperties
            if (props.messageTs) {
              await this.slackMessage.addReaction(props.channelId, props.messageTs, 'question')
            }

            logger.debug(
              { taskId, provider: 'slack', toolName: pendingPayload.toolName },
              'Sent pending response notification',
            )
          }
          break
      }
    } catch (error) {
      logger.error(
        { err: error, taskId, toolName: pendingPayload.toolName },
        'Failed to send pending response notification to channel',
      )
    }
  }

  /**
   * Format a pending response into a human-readable message for the channel.
   */
  private formatPendingResponseMessage(payload: PendingResponsePayload, taskUrl: string): string {
    const toolName = payload.toolName
    const toolInput = payload.toolInput

    if (toolName === 'AskUserQuestion') {
      // Extract questions from toolInput (use bracket notation for index signature)
      const questionsRaw = toolInput['questions']
      const questions =
        (questionsRaw as Array<{ question: string; options?: Array<{ label: string }> }>) || []

      if (questions.length === 0) {
        return `The agent needs your input.\n\nView task: ${taskUrl}`
      }

      const questionLines = questions.map((q, idx) => {
        const optionLabels = q.options?.map((o) => o.label).join(', ') || ''
        return `${idx + 1}. ${q.question}${optionLabels ? `\n   Options: ${optionLabels}` : ''}`
      })

      return `*The agent has a question for you:*\n\n${questionLines.join('\n\n')}\n\nReply to this message with your answer, or view the task: ${taskUrl}`
    }

    if (toolName === 'ExitPlanMode') {
      return `*The agent has created a plan for your approval.*\n\nReply with "approve" or "yes" to proceed, or provide feedback.\n\nView task: ${taskUrl}`
    }

    return `The agent is waiting for your input.\n\nView task: ${taskUrl}`
  }

  /**
   * Get the frontend URL for a task.
   */
  private getTaskUrl(taskId: string): string {
    const frontendUrl = (process.env['FRONTEND_URL'] || 'http://localhost:8000').replace(/\/$/, '')
    return `${frontendUrl}/tasks/${taskId}`
  }
}
