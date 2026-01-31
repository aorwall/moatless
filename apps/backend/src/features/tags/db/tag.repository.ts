import { randomUUID } from 'node:crypto'
import type { Tag } from '@moatless/types'
import { eq, inArray } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { logger } from '../../../utils/logger'
import { selectToTag, tags, tagToInsert, taskTags } from './tag.schema'

/**
 * Repository for tag persistence using Drizzle ORM.
 * Provides get-or-create semantics and task-tag association management.
 */
@injectable()
export class TagRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Find a tag by its URL-safe name.
   */
  async findByName(name: string): Promise<Tag | null> {
    const rows = await this.db.drizzle.select().from(tags).where(eq(tags.name, name)).limit(1)

    const row = rows[0]
    return row ? selectToTag(row) : null
  }

  /**
   * Get or create a tag by name.
   * If a tag with the given name exists, return it.
   * Otherwise, create a new tag and return it.
   */
  async getOrCreateByName(name: string, description?: string): Promise<Tag> {
    const existing = await this.findByName(name)
    if (existing) {
      return existing
    }

    const tag: Tag = {
      tagId: randomUUID(),
      name,
      description,
    }

    await this.db.drizzle.insert(tags).values(tagToInsert(tag))
    logger.debug({ tagId: tag.tagId, name }, 'Tag created')
    return tag
  }

  /**
   * Get all tags associated with a task.
   */
  async getTagsForTask(taskId: string): Promise<Tag[]> {
    const rows = await this.db.drizzle
      .select({
        tagId: tags.tagId,
        name: tags.name,
        description: tags.description,
        id: tags.id,
        createdAt: tags.createdAt,
      })
      .from(taskTags)
      .innerJoin(tags, eq(taskTags.tagId, tags.tagId))
      .where(eq(taskTags.taskId, taskId))

    return rows.map(selectToTag)
  }

  /**
   * Batch-load tags for multiple tasks.
   * Returns a Map from taskId to array of Tags.
   */
  async getTagsForTasks(taskIds: string[]): Promise<Map<string, Tag[]>> {
    if (taskIds.length === 0) {
      return new Map()
    }

    const rows = await this.db.drizzle
      .select({
        taskId: taskTags.taskId,
        tagId: tags.tagId,
        name: tags.name,
        description: tags.description,
        id: tags.id,
        createdAt: tags.createdAt,
      })
      .from(taskTags)
      .innerJoin(tags, eq(taskTags.tagId, tags.tagId))
      .where(inArray(taskTags.taskId, taskIds))

    const result = new Map<string, Tag[]>()
    for (const row of rows) {
      const existing = result.get(row.taskId) || []
      existing.push(selectToTag(row))
      result.set(row.taskId, existing)
    }
    return result
  }

  /**
   * Replace all tags for a task with the given tag IDs.
   */
  async setTagsForTask(taskId: string, tagIds: string[]): Promise<void> {
    await this.db.transaction(async (tx) => {
      await tx.delete(taskTags).where(eq(taskTags.taskId, taskId))

      if (tagIds.length > 0) {
        await tx.insert(taskTags).values(tagIds.map((tagId) => ({ taskId, tagId })))
      }
    })

    logger.debug({ taskId, tagCount: tagIds.length }, 'Task tags replaced')
  }

  /**
   * Add a single tag to a task (upsert - no error if already exists).
   */
  async addTagToTask(taskId: string, tagId: string): Promise<void> {
    await this.db.drizzle
      .insert(taskTags)
      .values({ taskId, tagId })
      .onConflictDoNothing({
        target: [taskTags.taskId, taskTags.tagId],
      })
  }
}
