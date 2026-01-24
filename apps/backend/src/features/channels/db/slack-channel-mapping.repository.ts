import { and, eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { logger } from '../../../utils/logger'
import type { SlackChannelMapping } from './slack-channel-mapping.schema'
import {
  selectToSlackChannelMapping,
  slackChannelMappings,
  slackChannelMappingToInsert,
} from './slack-channel-mapping.schema'

/**
 * Repository for Slack channel mapping persistence using Drizzle ORM.
 * Provides type-safe CRUD operations for channel-to-repository mappings.
 */
@injectable()
export class SlackChannelMappingRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Create a new Slack channel mapping.
   */
  async create(mapping: SlackChannelMapping): Promise<SlackChannelMapping> {
    const insert = slackChannelMappingToInsert(mapping)
    const result = await this.db.drizzle.insert(slackChannelMappings).values(insert).returning()

    logger.debug(
      { channelId: mapping.channelId, teamId: mapping.teamId, repositoryId: mapping.repositoryId },
      'Slack channel mapping created',
    )

    const row = result[0]
    if (!row) {
      throw new Error('Failed to create Slack channel mapping - no row returned')
    }

    return selectToSlackChannelMapping(row)
  }

  /**
   * Find a Slack channel mapping by channel ID and team ID.
   */
  async findByChannel(channelId: string, teamId: string): Promise<SlackChannelMapping | null> {
    const rows = await this.db.drizzle
      .select()
      .from(slackChannelMappings)
      .where(
        and(eq(slackChannelMappings.channelId, channelId), eq(slackChannelMappings.teamId, teamId)),
      )
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToSlackChannelMapping(row)
  }

  /**
   * Find a Slack channel mapping by ID.
   */
  async findById(id: number): Promise<SlackChannelMapping | null> {
    const rows = await this.db.drizzle
      .select()
      .from(slackChannelMappings)
      .where(eq(slackChannelMappings.id, id))
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToSlackChannelMapping(row)
  }

  /**
   * Find all Slack channel mappings for a repository.
   */
  async findByRepositoryId(repositoryId: string): Promise<SlackChannelMapping[]> {
    const rows = await this.db.drizzle
      .select()
      .from(slackChannelMappings)
      .where(eq(slackChannelMappings.repositoryId, repositoryId))

    return rows.map(selectToSlackChannelMapping)
  }

  /**
   * List all Slack channel mappings.
   */
  async list(): Promise<SlackChannelMapping[]> {
    const rows = await this.db.drizzle.select().from(slackChannelMappings)

    return rows.map(selectToSlackChannelMapping)
  }

  /**
   * Update a Slack channel mapping.
   */
  async update(
    channelId: string,
    teamId: string,
    updates: Partial<Pick<SlackChannelMapping, 'repositoryId'>>,
  ): Promise<SlackChannelMapping | null> {
    const result = await this.db.drizzle
      .update(slackChannelMappings)
      .set({
        ...updates,
        updatedAt: new Date(),
      })
      .where(
        and(eq(slackChannelMappings.channelId, channelId), eq(slackChannelMappings.teamId, teamId)),
      )
      .returning()

    const row = result[0]
    if (!row) {
      return null
    }

    logger.debug({ channelId, teamId }, 'Slack channel mapping updated')

    return selectToSlackChannelMapping(row)
  }

  /**
   * Delete a Slack channel mapping.
   */
  async delete(channelId: string, teamId: string): Promise<boolean> {
    const result = await this.db.drizzle
      .delete(slackChannelMappings)
      .where(
        and(eq(slackChannelMappings.channelId, channelId), eq(slackChannelMappings.teamId, teamId)),
      )
      .returning()

    if (result.length > 0) {
      logger.debug({ channelId, teamId }, 'Slack channel mapping deleted')
      return true
    }

    return false
  }
}
