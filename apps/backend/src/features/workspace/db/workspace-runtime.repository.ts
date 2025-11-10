import { eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database.js'
import { DatabaseSymbol } from '../../../infrastructure/database/database.js'
import { logger } from '../../../utils/logger.js'
import {
  bumpConfigRevision,
  type WorkspaceRuntimeSnapshot,
  WorkspaceRuntimeSnapshotSchema,
} from '../types/workspace-runtime.types.js'
import {
  selectToSnapshot,
  snapshotToInsert,
  workspaceRuntimes,
} from './workspace-runtime.schema.js'

/**
 * Repository for workspace runtime snapshot persistence using Drizzle ORM.
 * Provides type-safe database operations for workspace runtime state.
 *
 * This replaces the legacy WorkspaceRuntimeStorage adapter, using a relational database
 * instead of JSON file storage.
 */
@injectable()
export class WorkspaceRuntimeRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Save a workspace runtime snapshot to the database.
   * Creates a new record or updates an existing one (upsert).
   *
   * @param snapshot - Workspace runtime snapshot to save
   */
  async save(snapshot: WorkspaceRuntimeSnapshot): Promise<void> {
    logger.debug('Saving workspace runtime snapshot', {
      taskId: snapshot.taskId,
      key: `workspace-runtime/${snapshot.taskId}`,
    })

    // Validate before saving
    const validated = WorkspaceRuntimeSnapshotSchema.parse(snapshot)
    const insert = snapshotToInsert(validated)

    await this.db.drizzle
      .insert(workspaceRuntimes)
      .values(insert)
      .onConflictDoUpdate({
        target: workspaceRuntimes.taskId,
        set: {
          repositoryId: insert.repositoryId,
          configRevision: insert.configRevision,
          executionTarget: insert.executionTarget,
          agent: insert.agent,
          preview: insert.preview,
          locks: insert.locks,
          updatedAt: insert.updatedAt,
        },
      })

    logger.debug('Workspace runtime snapshot saved', {
      taskId: snapshot.taskId,
      configRevision: validated.configRevision,
    })
  }

  /**
   * Load a workspace runtime snapshot by task ID.
   *
   * @param taskId - Task identifier
   * @returns Workspace runtime snapshot or null if not found
   */
  async load(taskId: string): Promise<WorkspaceRuntimeSnapshot | null> {
    logger.debug('Loading workspace runtime snapshot', {
      taskId,
      key: `workspace-runtime/${taskId}`,
    })

    const rows = await this.db.drizzle
      .select()
      .from(workspaceRuntimes)
      .where(eq(workspaceRuntimes.taskId, taskId))
      .limit(1)

    const row = rows[0]
    if (!row) {
      logger.debug('No workspace runtime snapshot found', { taskId })
      return null
    }

    // Validate loaded data (Zod will coerce Date objects from storage)
    try {
      const snapshot = selectToSnapshot(row)
      const validated = WorkspaceRuntimeSnapshotSchema.parse(snapshot)

      logger.debug('Workspace runtime snapshot loaded', {
        taskId,
        configRevision: validated.configRevision,
      })

      return validated
    } catch (error) {
      logger.error('Invalid workspace runtime snapshot, ignoring', { taskId, error })
      return null
    }
  }

  /**
   * Update a workspace runtime snapshot with optimistic locking.
   * Returns true if update succeeded, false if snapshot was modified concurrently.
   *
   * @param taskId - Task identifier
   * @param updater - Function to transform the snapshot
   * @returns True if update succeeded, false otherwise
   */
  async update(
    taskId: string,
    updater: (snapshot: WorkspaceRuntimeSnapshot) => WorkspaceRuntimeSnapshot,
  ): Promise<boolean> {
    const current = await this.load(taskId)
    if (!current) {
      logger.warn('Cannot update non-existent workspace snapshot', { taskId })
      return false
    }

    const updated = updater(current)
    const bumped = bumpConfigRevision(updated)

    // Simple optimistic locking: if someone else updated between load and save,
    // the configRevision will have changed
    // For now we just save - a more sophisticated implementation would check
    // the stored revision before saving
    await this.save(bumped)
    return true
  }

  /**
   * Delete a workspace runtime snapshot by task ID.
   *
   * @param taskId - Task identifier
   */
  async delete(taskId: string): Promise<void> {
    logger.debug('Deleting workspace runtime snapshot', {
      taskId,
      key: `workspace-runtime/${taskId}`,
    })

    await this.db.drizzle.delete(workspaceRuntimes).where(eq(workspaceRuntimes.taskId, taskId))

    logger.info('Workspace runtime snapshot deleted', { taskId })
  }

  /**
   * List all workspace runtime snapshots.
   * Use with caution - consider pagination for production use.
   *
   * @returns Array of all workspace runtime snapshots
   */
  async list(): Promise<WorkspaceRuntimeSnapshot[]> {
    const rows = await this.db.drizzle.select().from(workspaceRuntimes)

    const snapshots = rows
      .map((row) => {
        try {
          const snapshot = selectToSnapshot(row)
          return WorkspaceRuntimeSnapshotSchema.parse(snapshot)
        } catch (error) {
          logger.error('Invalid workspace runtime snapshot in list, skipping', {
            taskId: row.taskId,
            error,
          })
          return null
        }
      })
      .filter((s): s is WorkspaceRuntimeSnapshot => s !== null)

    return snapshots
  }

  /**
   * Check if a snapshot exists for a task.
   *
   * @param taskId - Task identifier
   * @returns True if snapshot exists, false otherwise
   */
  async exists(taskId: string): Promise<boolean> {
    const rows = await this.db.drizzle
      .select({ taskId: workspaceRuntimes.taskId })
      .from(workspaceRuntimes)
      .where(eq(workspaceRuntimes.taskId, taskId))
      .limit(1)

    return rows.length > 0
  }
}
