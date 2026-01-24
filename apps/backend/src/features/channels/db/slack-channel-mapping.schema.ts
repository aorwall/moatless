import { sql } from 'drizzle-orm'
import { pgTable, serial, text, timestamp, uniqueIndex } from 'drizzle-orm/pg-core'

/**
 * Domain model for Slack channel mappings.
 */
export interface SlackChannelMapping {
  id?: number
  channelId: string
  teamId: string
  repositoryId: string
  createdBy: string
  createdAt: Date
  updatedAt: Date
}

/**
 * Slack channel mappings table schema (PostgreSQL).
 * Maps Slack channels to default repositories for streamlined task creation.
 * Users can mention @moatless without specifying repo: if the channel has a default.
 */
export const slackChannelMappings = pgTable(
  'slack_channel_mappings',
  {
    id: serial('id').primaryKey(),
    channelId: text('channel_id').notNull(),
    teamId: text('team_id').notNull(),
    repositoryId: text('repository_id').notNull(),
    createdBy: text('created_by').notNull(),
    createdAt: timestamp('created_at').notNull().default(sql`now()`),
    updatedAt: timestamp('updated_at').notNull().default(sql`now()`),
  },
  (table) => ({
    channelTeamIdx: uniqueIndex('slack_channel_mappings_channel_team_idx').on(
      table.channelId,
      table.teamId,
    ),
  }),
)

export type SlackChannelMappingInsert = typeof slackChannelMappings.$inferInsert
export type SlackChannelMappingSelect = typeof slackChannelMappings.$inferSelect

/**
 * Convert domain model to database insert format.
 */
export function slackChannelMappingToInsert(
  mapping: SlackChannelMapping,
): SlackChannelMappingInsert {
  return {
    channelId: mapping.channelId,
    teamId: mapping.teamId,
    repositoryId: mapping.repositoryId,
    createdBy: mapping.createdBy,
    createdAt: mapping.createdAt,
    updatedAt: mapping.updatedAt,
  }
}

/**
 * Convert database row to domain model.
 */
export function selectToSlackChannelMapping(row: SlackChannelMappingSelect): SlackChannelMapping {
  return {
    id: row.id,
    channelId: row.channelId,
    teamId: row.teamId,
    repositoryId: row.repositoryId,
    createdBy: row.createdBy,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}
