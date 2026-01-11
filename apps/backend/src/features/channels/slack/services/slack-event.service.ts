import type { Task } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../../utils/logger'
import { RepositoryRepository } from '../../../repositories/db/repository.repository'
import { TaskRepository } from '../../../tasks/db/task.repository'
import { TaskService } from '../../../tasks/services/task.service'
import type { SlackThreadProperties } from '../../db/channel-thread.schema'
import { ThreadTrackerService } from '../../services/thread-tracker.service'
import { UserResolverService } from '../../services/user-resolver.service'

/**
 * Slack event payload structure.
 */
export interface SlackEvent {
  type: string
  user: string
  text: string
  ts: string
  channel: string
  thread_ts?: string
  team?: string
}

/**
 * Slack event wrapper with metadata.
 */
export interface SlackEventWrapper {
  type: string
  token?: string
  team_id: string
  api_app_id: string
  event: SlackEvent
  event_id: string
  event_time: number
  challenge?: string
}

/**
 * Result of processing a Slack event.
 */
export interface SlackEventResult {
  success: boolean
  taskId?: string
  isNewTask: boolean
  error?: string
  shouldReply: boolean
  replyMessage?: string
}

/**
 * Service for processing Slack events.
 * Handles app_mention events to create tasks or send messages.
 */
@injectable()
export class SlackEventService {
  constructor(
    @inject(UserResolverService) private userResolver: UserResolverService,
    @inject(ThreadTrackerService) private threadTracker: ThreadTrackerService,
    @inject(TaskService) private taskService: TaskService,
    @inject(TaskRepository) private taskRepository: TaskRepository,
    @inject(RepositoryRepository) private repositoryRepository: RepositoryRepository,
  ) {}

  /**
   * Process a Slack event.
   *
   * Flow:
   * 1. Handle URL verification challenge
   * 2. For app_mention events:
   *    a. Fetch Slack user email
   *    b. Resolve system user by email
   *    c. Extract repository from message
   *    d. Check if this is a thread reply to existing task
   *    e. Create task or send message
   *
   * @param eventWrapper - The Slack event wrapper
   * @returns Processing result
   */
  async processEvent(eventWrapper: SlackEventWrapper): Promise<SlackEventResult> {
    // Handle URL verification challenge
    if (eventWrapper.type === 'url_verification') {
      return {
        success: true,
        isNewTask: false,
        shouldReply: false,
      }
    }

    const event = eventWrapper.event
    if (!event || event.type !== 'app_mention') {
      return {
        success: true,
        isNewTask: false,
        shouldReply: false,
      }
    }

    logger.info(
      { channel: event.channel, user: event.user, text: event.text?.slice(0, 100) },
      'Processing Slack app_mention event',
    )

    // 1. Fetch Slack user email and resolve to system user
    const slackUserEmail = await this.fetchSlackUserEmail(event.user, eventWrapper.team_id)
    if (!slackUserEmail) {
      logger.warn({ slackUser: event.user }, 'Could not fetch Slack user email')
      return {
        success: false,
        isNewTask: false,
        shouldReply: true,
        error:
          'Could not identify your Moatless account. Please ensure your Slack email matches your Moatless account email.',
        replyMessage:
          'Could not identify your Moatless account. Please ensure your Slack email matches your Moatless account email.',
      }
    }

    const user = await this.userResolver.resolveByEmail(slackUserEmail)
    if (!user) {
      logger.warn({ email: slackUserEmail }, 'User not found by email')
      return {
        success: false,
        isNewTask: false,
        shouldReply: true,
        error: `No Moatless account found for email ${slackUserEmail}`,
        replyMessage: `No Moatless account found for email ${slackUserEmail}. Please register first.`,
      }
    }

    // 2. Extract repository from message: @moatless repo:my-repo message
    const { repository: repositoryName, message } = this.parseMessage(event.text)
    if (!repositoryName) {
      return {
        success: false,
        isNewTask: false,
        shouldReply: true,
        error: 'No repository specified',
        replyMessage: 'Please specify a repository: `@moatless repo:my-repo your message here`',
      }
    }

    const repository = await this.repositoryRepository.findByNameAndUser(repositoryName, user.id)
    if (!repository) {
      return {
        success: false,
        isNewTask: false,
        shouldReply: true,
        error: `Repository '${repositoryName}' not found`,
        replyMessage: `Repository '${repositoryName}' not found. Please check the name and try again.`,
      }
    }

    // 3. Check if this is a reply to an existing task thread
    const threadTs = event.thread_ts || event.ts
    const existingTask = await this.findExistingTask(threadTs)

    if (existingTask) {
      // Send message to existing task
      await this.taskService.sendMessage(
        existingTask.taskId,
        message,
        true, // resume
      )

      // Update thread timestamp
      const thread = await this.threadTracker.findThreadByTaskId(existingTask.taskId, 'slack')
      if (thread?.id) {
        await this.threadTracker.updateLastMessageAt(thread.id)
      }

      logger.info(
        { taskId: existingTask.taskId, channel: event.channel },
        'Message sent to existing task from Slack',
      )

      return {
        success: true,
        taskId: existingTask.taskId,
        isNewTask: false,
        shouldReply: true,
        replyMessage: `Message sent to task \`${existingTask.shortId || existingTask.taskId.slice(0, 8)}\``,
      }
    }

    // 4. Create new task
    const task = await this.taskService.createTask({
      agentType: 'claude-code',
      repositoryId: repository.id,
      message,
      start: true,
    })

    // Create Slack thread for tracking
    const threadProperties: SlackThreadProperties = {
      channelId: event.channel,
      teamId: eventWrapper.team_id,
      userId: event.user,
    }

    await this.threadTracker.createSlackThread(task.taskId, threadTs, threadProperties)

    logger.info({ taskId: task.taskId, channel: event.channel }, 'New task created from Slack')

    return {
      success: true,
      taskId: task.taskId,
      isNewTask: true,
      shouldReply: true,
      replyMessage: `Task \`${task.shortId || task.taskId.slice(0, 8)}\` created! I'll post updates here as the agent works.`,
    }
  }

  /**
   * Parse the message text to extract repository and message.
   * Format: @moatless repo:my-repo message here
   */
  private parseMessage(text: string): { repository: string | null; message: string } {
    // Remove the @mention at the start
    const cleanText = text.replace(/^<@[A-Z0-9]+>\s*/i, '').trim()

    // Look for repo:name pattern
    const repoMatch = cleanText.match(/\brepo:([a-z0-9_-]+)/i)
    const repository = repoMatch?.[1]?.toLowerCase() || null

    // Remove repo: pattern from message
    const message = cleanText.replace(/\brepo:[a-z0-9_-]+\s*/i, '').trim()

    return { repository, message }
  }

  /**
   * Find an existing task from Slack thread.
   */
  private async findExistingTask(threadTs: string): Promise<Task | null> {
    const thread = await this.threadTracker.findSlackThread(threadTs)
    if (!thread) {
      return null
    }

    const task = await this.taskRepository.get(thread.taskId)
    if (task) {
      logger.debug({ threadTs }, 'Found task via Slack thread')
      return task as Task
    }

    return null
  }

  /**
   * Fetch Slack user's email address via Slack API.
   * Requires users:read.email scope.
   */
  private async fetchSlackUserEmail(userId: string, _teamId: string): Promise<string | null> {
    const botToken = process.env['SLACK_BOT_TOKEN']
    if (!botToken) {
      logger.warn('SLACK_BOT_TOKEN not configured - cannot fetch user email')
      return null
    }

    try {
      const response = await fetch(`https://slack.com/api/users.info?user=${userId}`, {
        headers: {
          Authorization: `Bearer ${botToken}`,
        },
      })

      const data = (await response.json()) as {
        ok: boolean
        user?: { profile?: { email?: string } }
        error?: string
      }

      if (!data.ok) {
        logger.warn({ error: data.error, userId }, 'Slack API error fetching user info')
        return null
      }

      return data.user?.profile?.email || null
    } catch (error) {
      logger.error({ err: error, userId }, 'Error fetching Slack user email')
      return null
    }
  }
}
