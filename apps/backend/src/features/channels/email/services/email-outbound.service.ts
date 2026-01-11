import { inject, injectable } from 'inversify'
import { logger } from '../../../../utils/logger'
import { TaskRepository } from '../../../tasks/db/task.repository'
import type { EmailThreadProperties } from '../../db/channel-thread.schema'
import { ThreadTrackerService } from '../../services/thread-tracker.service'
import { formatReplySubject } from '../utils/email-parser.utils'

/**
 * Configuration for SendGrid email sending.
 */
interface SendGridConfig {
  apiKey: string
  fromEmail: string
  fromName: string
}

/**
 * Service for sending outbound emails via SendGrid.
 * Handles sending replies to email threads when agents complete tasks.
 */
@injectable()
export class EmailOutboundService {
  private config: SendGridConfig | null = null

  constructor(
    @inject(ThreadTrackerService) private threadTracker: ThreadTrackerService,
    @inject(TaskRepository) private taskRepository: TaskRepository,
  ) {
    this.initializeConfig()
  }

  /**
   * Initialize SendGrid configuration from environment.
   * Uses new EMAIL_* vars with fallback to old SENDGRID_* vars for backward compatibility.
   */
  private initializeConfig(): void {
    const apiKey = process.env['SENDGRID_API_KEY']
    if (!apiKey) {
      logger.warn('SENDGRID_API_KEY not configured - email sending disabled')
      return
    }

    this.config = {
      apiKey,
      // Use new EMAIL_* vars with fallback to old SENDGRID_* vars
      fromEmail:
        process.env['EMAIL_FROM_ADDRESS'] ||
        process.env['SENDGRID_FROM_EMAIL'] ||
        'noreply@inbox.moatless.ai',
      fromName: process.env['EMAIL_FROM_NAME'] || process.env['SENDGRID_FROM_NAME'] || 'Moatless',
    }

    logger.info('Email outbound service initialized')
  }

  /**
   * Check if email sending is enabled.
   */
  isEnabled(): boolean {
    return this.config !== null
  }

  /**
   * Send a reply email for a task.
   * Called when an agent completes an iteration.
   *
   * @param taskId - The task ID
   * @param message - The message content to send
   */
  async sendTaskReply(taskId: string, message: string): Promise<void> {
    if (!this.config) {
      logger.debug({ taskId }, 'Email sending disabled - skipping reply')
      return
    }

    // Find email thread for this task
    const thread = await this.threadTracker.findThreadByTaskId(taskId, 'email')
    if (!thread) {
      logger.debug({ taskId }, 'No email thread found for task - skipping reply')
      return
    }

    const task = await this.taskRepository.get(taskId)
    if (!task) {
      logger.warn({ taskId }, 'Task not found - skipping email reply')
      return
    }

    const properties = thread.properties as EmailThreadProperties
    const shortId = task.shortId || taskId.slice(0, 8)

    try {
      await this.sendEmail({
        to: properties.recipientEmail,
        from: properties.replyFromEmail,
        subject: formatReplySubject(shortId, properties.initialSubject),
        text: message,
        inReplyTo: thread.externalThreadId,
        references: thread.externalThreadId,
        taskShortId: shortId,
      })

      // Update thread timestamp
      if (thread.id) {
        await this.threadTracker.updateLastMessageAt(thread.id)
      }

      logger.info({ taskId, to: properties.recipientEmail }, 'Email reply sent')
    } catch (error) {
      logger.error({ err: error, taskId }, 'Failed to send email reply')
    }
  }

  /**
   * Send an error reply email.
   * Used when processing fails (e.g., repository not found).
   *
   * @param to - Recipient email address
   * @param from - From address (original recipient of the inbound email)
   * @param subject - Original subject line
   * @param errorMessage - The error message to send
   * @param messageId - Original message ID for threading
   */
  async sendErrorReply(
    to: string,
    from: string,
    subject: string,
    errorMessage: string,
    messageId?: string,
  ): Promise<void> {
    if (!this.config) {
      logger.debug('Email sending disabled - skipping error reply')
      return
    }

    try {
      await this.sendEmail({
        to,
        from,
        subject: `Re: ${subject}`,
        text: errorMessage,
        ...(messageId && { inReplyTo: messageId, references: messageId }),
      })
      logger.info({ to, from }, 'Error reply sent')
    } catch (error) {
      logger.error({ err: error, to }, 'Failed to send error reply')
    }
  }

  /**
   * Send an email via SendGrid API.
   */
  private async sendEmail(params: {
    to: string
    from?: string
    subject: string
    text: string
    inReplyTo?: string
    references?: string
    taskShortId?: string
  }): Promise<void> {
    if (!this.config) {
      throw new Error('Email sending not configured')
    }

    const headers: Record<string, string> = {}
    if (params.inReplyTo) {
      headers['In-Reply-To'] = `<${params.inReplyTo}>`
    }
    if (params.references) {
      headers['References'] = `<${params.references}>`
    }
    if (params.taskShortId) {
      headers['X-Task-ShortId'] = params.taskShortId
    }

    // Use provided from address or fall back to configured default
    const fromEmail = params.from ?? this.config.fromEmail

    const payload = {
      personalizations: [
        {
          to: [{ email: params.to }],
        },
      ],
      from: {
        email: fromEmail,
        name: this.config.fromName,
      },
      subject: params.subject,
      content: [
        {
          type: 'text/plain',
          value: params.text,
        },
      ],
      headers,
    }

    const response = await fetch('https://api.sendgrid.com/v3/mail/send', {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${this.config.apiKey}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(payload),
    })

    if (!response.ok) {
      const error = await response.text()
      throw new Error(`SendGrid API error: ${response.status} - ${error}`)
    }

    // Log SendGrid message ID for tracking
    const messageId = response.headers.get('X-Message-Id')
    logger.info(
      {
        status: response.status,
        messageId,
        to: params.to,
        from: fromEmail,
        subject: params.subject,
      },
      'SendGrid accepted email',
    )
  }
}
