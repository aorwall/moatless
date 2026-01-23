import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { ChannelThreadRepository } from '../db/channel-thread.repository'
import type {
  ChannelProvider,
  ChannelThread,
  ChannelThreadProperties,
  EmailThreadProperties,
  SlackThreadProperties,
} from '../db/channel-thread.schema'

/**
 * Service for tracking message threads across external channels.
 * Provides a unified interface for creating and finding threads.
 */
@injectable()
export class ThreadTrackerService {
  constructor(
    @inject(ChannelThreadRepository)
    private threadRepository: ChannelThreadRepository,
  ) {}

  /**
   * Create a new email thread for a task.
   *
   * @param taskId - The task ID this thread belongs to
   * @param messageId - The email Message-ID header
   * @param properties - Email-specific thread properties
   * @returns The created thread
   */
  async createEmailThread(
    taskId: string,
    messageId: string,
    properties: EmailThreadProperties,
  ): Promise<ChannelThread> {
    const thread: ChannelThread = {
      taskId,
      provider: 'email',
      externalThreadId: messageId,
      properties,
      lastMessageAt: new Date(),
      createdAt: new Date(),
    }

    const created = await this.threadRepository.create(thread)
    logger.info({ taskId, messageId }, 'Email thread created')
    return created
  }

  /**
   * Create a new Slack thread for a task.
   *
   * Handles duplicate key errors gracefully - this can happen when Slack
   * retries events quickly, causing concurrent thread creation attempts.
   *
   * @param taskId - The task ID this thread belongs to
   * @param threadTs - The Slack thread timestamp
   * @param properties - Slack-specific thread properties
   * @returns The created thread, or existing thread if already created
   */
  async createSlackThread(
    taskId: string,
    threadTs: string,
    properties: SlackThreadProperties,
  ): Promise<ChannelThread> {
    const thread: ChannelThread = {
      taskId,
      provider: 'slack',
      externalThreadId: threadTs,
      properties,
      lastMessageAt: new Date(),
      createdAt: new Date(),
    }

    try {
      const created = await this.threadRepository.create(thread)
      logger.info({ taskId, threadTs }, 'Slack thread created')
      return created
    } catch (error) {
      // Handle duplicate key - thread already exists (Slack retry race condition)
      if (error instanceof Error && error.message.includes('duplicate key')) {
        logger.info({ taskId, threadTs }, 'Slack thread already exists (likely retry)')
        const existing = await this.findSlackThread(threadTs)
        if (existing) {
          return existing
        }
        // If we can't find it after duplicate error, re-throw
        throw error
      }
      throw error
    }
  }

  /**
   * Find a thread by email Message-ID or References header.
   * Used for email reply threading.
   *
   * @param messageId - The Message-ID to search for
   * @returns The thread if found, null otherwise
   */
  findEmailThread(messageId: string): Promise<ChannelThread | null> {
    return this.threadRepository.findByExternalId('email', messageId)
  }

  /**
   * Find a thread by Slack thread timestamp.
   *
   * @param threadTs - The Slack thread timestamp
   * @returns The thread if found, null otherwise
   */
  findSlackThread(threadTs: string): Promise<ChannelThread | null> {
    return this.threadRepository.findByExternalId('slack', threadTs)
  }

  /**
   * Find a thread by task ID and provider.
   * Used for sending replies back to the originating service.
   *
   * @param taskId - The task ID
   * @param provider - The channel provider
   * @returns The thread if found, null otherwise
   */
  findThreadByTaskId(taskId: string, provider: ChannelProvider): Promise<ChannelThread | null> {
    return this.threadRepository.findByTaskId(taskId, provider)
  }

  /**
   * Find any channel thread for a task (regardless of provider).
   * Returns the thread with its provider type for notification dispatch.
   *
   * @param taskId - The task ID
   * @returns The thread if found, null otherwise
   */
  findAnyThreadByTaskId(taskId: string): Promise<ChannelThread | null> {
    return this.threadRepository.findAnyByTaskId(taskId)
  }

  /**
   * Update the last message timestamp for a thread.
   *
   * @param threadId - The thread ID
   */
  async updateLastMessageAt(threadId: number): Promise<void> {
    await this.threadRepository.updateLastMessageAt(threadId, new Date())
  }

  /**
   * Update the properties for a thread.
   *
   * @param threadId - The thread ID
   * @param properties - The new properties
   */
  async updateProperties(threadId: number, properties: ChannelThreadProperties): Promise<void> {
    await this.threadRepository.updateProperties(threadId, properties)
  }
}
