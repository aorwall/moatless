import type { Tag } from '@moatless/types'
import { sql } from 'drizzle-orm'
import { index, pgTable, serial, text, timestamp, uniqueIndex } from 'drizzle-orm/pg-core'
import { tasks } from '../../tasks/db/task.schema'

/**
 * Tags table schema (PostgreSQL).
 * Stores tag definitions with URL-safe names.
 */
export const tags = pgTable(
  'tags',
  {
    id: serial('id').primaryKey(),
    tagId: text('tag_id').notNull(),
    name: text('name').notNull(),
    description: text('description'),
    createdAt: timestamp('created_at').notNull().default(sql`now()`),
  },
  (table) => ({
    tagIdIdx: uniqueIndex('tags_tag_id_idx').on(table.tagId),
    nameIdx: uniqueIndex('tags_name_idx').on(table.name),
  }),
)

/**
 * Task-Tags join table (PostgreSQL).
 * Associates tasks with tags (many-to-many).
 */
export const taskTags = pgTable(
  'task_tags',
  {
    id: serial('id').primaryKey(),
    taskId: text('task_id')
      .notNull()
      .references(() => tasks.taskId, { onDelete: 'cascade' }),
    tagId: text('tag_id')
      .notNull()
      .references(() => tags.tagId, { onDelete: 'cascade' }),
    createdAt: timestamp('created_at').notNull().default(sql`now()`),
  },
  (table) => ({
    taskTagUniqueIdx: uniqueIndex('task_tags_task_id_tag_id_idx').on(table.taskId, table.tagId),
    tagIdIdx: index('task_tags_tag_id_idx').on(table.tagId),
  }),
)

export type TagInsert = typeof tags.$inferInsert
export type TagSelect = typeof tags.$inferSelect
export type TaskTagInsert = typeof taskTags.$inferInsert
export type TaskTagSelect = typeof taskTags.$inferSelect

/**
 * Convert Tag domain model to database insert format
 */
export function tagToInsert(tag: Tag): TagInsert {
  return {
    tagId: tag.tagId,
    name: tag.name,
    description: tag.description ?? null,
  }
}

/**
 * Convert database row to Tag domain model
 */
export function selectToTag(row: TagSelect): Tag {
  return {
    tagId: row.tagId,
    name: row.name,
    description: row.description ?? undefined,
  }
}
