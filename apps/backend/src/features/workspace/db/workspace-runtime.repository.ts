import { and, eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database.js'
import { DatabaseSymbol } from '../../../infrastructure/database/database.js'
import {
  getSchemasFromEnv,
  type sqliteSchemas,
} from '../../../infrastructure/database/schemas/index.js'
import { logger } from '../../../utils/logger.js'
import {
  bumpSnapshotRevision,
  type WorkspaceLock,
  type WorkspaceRuntimeSnapshot,
  WorkspaceRuntimeSnapshotSchema,
} from '../types/workspace-runtime.types.js'
import {
  selectToSnapshot as pgSelectToSnapshot,
  snapshotToInsert as pgSnapshotToInsert,
} from './pg/workspace-runtime.schema.js'
import {
  selectToSnapshot as sqliteSelectToSnapshot,
  snapshotToInsert as sqliteSnapshotToInsert,
} from './workspace-runtime.schema.js'

// Get correct schema based on DATABASE_PROVIDER
// Cast to SQLite schema type for TypeScript - runtime uses correct provider
const isPostgres = process.env['DATABASE_PROVIDER'] === 'postgres'
const schemas = getSchemasFromEnv()
const workspaceRuntimes = schemas.workspaceRuntimes as typeof sqliteSchemas.workspaceRuntimes
const snapshotToInsert = isPostgres ? pgSnapshotToInsert : sqliteSnapshotToInsert
const selectToSnapshot = isPostgres ? pgSelectToSnapshot : sqliteSelectToSnapshot

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

  private getRowsAffected(result: unknown): number {
    if (!result || typeof result !== 'object') {
      return 0
    }
    const value = result as { rowsAffected?: number; rowCount?: number; changes?: number }
    if (typeof value.rowsAffected === 'number') {
      return value.rowsAffected
    }
    if (typeof value.rowCount === 'number') {
      return value.rowCount
    }
    if (typeof value.changes === 'number') {
      return value.changes
    }
    return 0
  }

  /**
   * Save a workspace runtime snapshot to the database.
   * Creates a new record or updates an existing one (upsert).
   *
   * @param snapshot - Workspace runtime snapshot to save
   */
  async save(snapshot: WorkspaceRuntimeSnapshot): Promise<void> {
    logger.debug(
      {
        taskId: snapshot.taskId,
        key: `workspace-runtime/${snapshot.taskId}`,
      },
      'Saving workspace runtime snapshot',
    )

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
          snapshotRevision: insert.snapshotRevision,
          executionTarget: insert.executionTarget,
          agent: insert.agent,
          preview: insert.preview,
          locks: insert.locks,
          updatedAt: insert.updatedAt,
        },
      })

    logger.debug(
      {
        taskId: snapshot.taskId,
        snapshotRevision: validated.snapshotRevision,
      },
      'Workspace runtime snapshot saved',
    )
  }

  /**
   * Insert a snapshot only if it does not exist.
   *
   * @returns true if inserted, false if a snapshot already exists.
   */
  async tryInsert(snapshot: WorkspaceRuntimeSnapshot): Promise<boolean> {
    const validated = WorkspaceRuntimeSnapshotSchema.parse(snapshot)
    const insert = snapshotToInsert(validated)

    // Use returning() to reliably detect if a row was inserted
    // PostgreSQL doesn't return rowsAffected for ON CONFLICT DO NOTHING
    const result = await this.db.drizzle
      .insert(workspaceRuntimes)
      .values(insert)
      .onConflictDoNothing()
      .returning({ taskId: workspaceRuntimes.taskId })

    const inserted = Array.isArray(result) && result.length > 0
    logger.debug(
      {
        taskId: snapshot.taskId,
        inserted,
        resultLength: Array.isArray(result) ? result.length : 0,
      },
      'tryInsert result',
    )
    return inserted
  }

  /**
   * Load a workspace runtime snapshot by task ID.
   *
   * @param taskId - Task identifier
   * @returns Workspace runtime snapshot or null if not found
   */
  async load(taskId: string): Promise<WorkspaceRuntimeSnapshot | null> {
    logger.debug(
      {
        taskId,
        key: `workspace-runtime/${taskId}`,
      },
      'Loading workspace runtime snapshot',
    )

    const rows = await this.db.drizzle
      .select()
      .from(workspaceRuntimes)
      .where(eq(workspaceRuntimes.taskId, taskId))
      .limit(1)

    const row = rows[0]
    if (!row) {
      logger.debug({ taskId }, 'No workspace runtime snapshot found')
      return null
    }

    // Validate loaded data (Zod will coerce Date objects from storage)
    try {
      const snapshot = selectToSnapshot(row)
      const validated = WorkspaceRuntimeSnapshotSchema.parse(snapshot)

      logger.debug(
        {
          taskId,
          snapshotRevision: validated.snapshotRevision,
        },
        'Workspace runtime snapshot loaded',
      )

      return validated
    } catch (error) {
      logger.error({ err: error, taskId }, 'Invalid workspace runtime snapshot, ignoring')
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
      logger.warn({ taskId }, 'Cannot update non-existent workspace snapshot')
      return false
    }

    const updated = updater(current)
    const bumped = bumpSnapshotRevision(updated)

    // Simple optimistic locking: if someone else updated between load and save,
    // the snapshotRevision will have changed
    // For now we just save - a more sophisticated implementation would check
    // the stored revision before saving
    await this.save(bumped)
    return true
  }

  /**
   * Update a snapshot only if the config revision matches the expected value.
   *
   * @returns true if the update succeeded, false if the revision changed.
   */
  async compareAndSwap(
    current: WorkspaceRuntimeSnapshot,
    updated: WorkspaceRuntimeSnapshot,
  ): Promise<boolean> {
    const bumped = bumpSnapshotRevision(updated)
    const insert = snapshotToInsert(bumped)

    const result = await this.db.drizzle
      .update(workspaceRuntimes)
      .set({
        repositoryId: insert.repositoryId,
        snapshotRevision: insert.snapshotRevision,
        executionTarget: insert.executionTarget,
        agent: insert.agent,
        preview: insert.preview,
        locks: insert.locks,
        createdAt: insert.createdAt,
        updatedAt: insert.updatedAt,
      })
      .where(
        and(
          eq(workspaceRuntimes.taskId, current.taskId),
          eq(workspaceRuntimes.snapshotRevision, current.snapshotRevision),
        ),
      )

    return this.getRowsAffected(result) > 0
  }

  /**
   * Attempt to acquire a lock on a snapshot.
   */
  async tryAcquireLock(taskId: string, lock: WorkspaceLock, ttlMs: number): Promise<boolean> {
    const snapshot = await this.load(taskId)
    if (!snapshot) {
      return false
    }

    const now = Date.now()
    const activeLocks = (snapshot.locks ?? []).filter(
      (existingLock) => now - existingLock.acquiredAt.getTime() < ttlMs,
    )

    if (activeLocks.some((existingLock) => existingLock.type === lock.type)) {
      return false
    }

    const updated: WorkspaceRuntimeSnapshot = {
      ...snapshot,
      locks: [...activeLocks, lock],
    }

    return await this.compareAndSwap(snapshot, updated)
  }

  /**
   * Release a lock by type and owner.
   */
  async releaseLock(taskId: string, type: WorkspaceLock['type'], owner: string): Promise<void> {
    await this.update(taskId, (snapshot) => {
      const locks = (snapshot.locks ?? []).filter(
        (lock) => !(lock.type === type && lock.owner === owner),
      )
      return {
        ...snapshot,
        locks: locks.length > 0 ? locks : undefined,
      }
    })
  }

  /**
   * Delete a workspace runtime snapshot by task ID.
   *
   * @param taskId - Task identifier
   */
  async delete(taskId: string): Promise<void> {
    logger.debug(
      {
        taskId,
        key: `workspace-runtime/${taskId}`,
      },
      'Deleting workspace runtime snapshot',
    )

    await this.db.drizzle.delete(workspaceRuntimes).where(eq(workspaceRuntimes.taskId, taskId))

    logger.info({ taskId }, 'Workspace runtime snapshot deleted')
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
          logger.error(
            { err: error, taskId: row.taskId },
            'Invalid workspace runtime snapshot in list, skipping',
          )
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
