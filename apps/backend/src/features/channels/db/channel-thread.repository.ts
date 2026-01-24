import { and, eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { logger } from '../../../utils/logger'
import type {
  ChannelProvider,
  ChannelThread,
  ChannelThreadProperties,
} from './channel-thread.schema'
import {
  channelThreads,
  channelThreadToInsert,
  selectToChannelThread,
} from './channel-thread.schema'

/**
 * Repository for channel thread persistence using Drizzle ORM.
 * Provides type-safe CRUD operations for channel threads.
 */
@injectable()
export class ChannelThreadRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Create a new channel thread.
   */
  async create(thread: ChannelThread): Promise<ChannelThread> {
    const insert = channelThreadToInsert(thread)
    const result = await this.db.drizzle.insert(channelThreads).values(insert).returning()

    logger.debug({ taskId: thread.taskId, provider: thread.provider }, 'Channel thread created')

    const row = result[0]
    if (!row) {
      throw new Error('Failed to create channel thread - no row returned')
    }

    return selectToChannelThread(row)
  }

  /**
   * Find a channel thread by provider and external thread ID.
   */
  async findByExternalId(
    provider: ChannelProvider,
    externalThreadId: string,
  ): Promise<ChannelThread | null> {
    const rows = await this.db.drizzle
      .select()
      .from(channelThreads)
      .where(
        and(
          eq(channelThreads.provider, provider),
          eq(channelThreads.externalThreadId, externalThreadId),
        ),
      )
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToChannelThread(row)
  }

  /**
   * Find a channel thread by task ID and provider.
   */
  async findByTaskId(taskId: string, provider: ChannelProvider): Promise<ChannelThread | null> {
    const rows = await this.db.drizzle
      .select()
      .from(channelThreads)
      .where(and(eq(channelThreads.taskId, taskId), eq(channelThreads.provider, provider)))
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToChannelThread(row)
  }

  /**
   * Find all channel threads for a task.
   */
  async findAllByTaskId(taskId: string): Promise<ChannelThread[]> {
    const rows = await this.db.drizzle
      .select()
      .from(channelThreads)
      .where(eq(channelThreads.taskId, taskId))

    return rows.map(selectToChannelThread)
  }

  /**
   * Find any channel thread for a task (regardless of provider).
   * Returns the first thread found, useful for notification dispatch.
   */
  async findAnyByTaskId(taskId: string): Promise<ChannelThread | null> {
    const rows = await this.db.drizzle
      .select()
      .from(channelThreads)
      .where(eq(channelThreads.taskId, taskId))
      .limit(1)

    const row = rows[0]
    return row ? selectToChannelThread(row) : null
  }

  /**
   * Update the last message timestamp for a thread.
   */
  async updateLastMessageAt(id: number, lastMessageAt: Date): Promise<void> {
    await this.db.drizzle
      .update(channelThreads)
      .set({ lastMessageAt })
      .where(eq(channelThreads.id, id))

    logger.debug({ id, lastMessageAt }, 'Channel thread updated')
  }

  /**
   * Update the properties for a thread.
   */
  async updateProperties(id: number, properties: ChannelThreadProperties): Promise<void> {
    await this.db.drizzle
      .update(channelThreads)
      .set({ properties })
      .where(eq(channelThreads.id, id))

    logger.debug({ id }, 'Channel thread properties updated')
  }

  /**
   * Delete a channel thread.
   */
  async delete(id: number): Promise<void> {
    await this.db.drizzle.delete(channelThreads).where(eq(channelThreads.id, id))

    logger.debug({ id }, 'Channel thread deleted')
  }

  /**
   * Delete all channel threads for a task.
   */
  async deleteByTaskId(taskId: string): Promise<void> {
    await this.db.drizzle.delete(channelThreads).where(eq(channelThreads.taskId, taskId))

    logger.debug({ taskId }, 'Channel threads deleted for task')
  }
}
