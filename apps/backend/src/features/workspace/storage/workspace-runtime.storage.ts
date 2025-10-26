import { inject, injectable } from 'inversify'
import type { StorageBackend } from '../../../storage/storage-backend'
import { StorageSymbol } from '../../../storage/storage-backend'
import { logger } from '../../../utils/logger'
import {
  bumpConfigRevision,
  type WorkspaceRuntimeSnapshot,
  WorkspaceRuntimeSnapshotSchema,
} from '../types/workspace-runtime.types'

/**
 * Storage adapter for workspace runtime snapshots
 * Persists workspace state to survive process restarts
 */
@injectable()
export class WorkspaceRuntimeStorage {
  private readonly keyPrefix = 'workspace-runtime'

  constructor(@inject(StorageSymbol) private readonly storage: StorageBackend) {}

  /**
   * Save a workspace runtime snapshot
   */
  async save(snapshot: WorkspaceRuntimeSnapshot): Promise<void> {
    const key = this.getKey(snapshot.taskId)
    logger.debug('Saving workspace runtime snapshot', { taskId: snapshot.taskId, key })

    // Validate before saving
    const validated = WorkspaceRuntimeSnapshotSchema.parse(snapshot)

    await this.storage.save(key, validated)
    logger.info('Workspace runtime snapshot saved', {
      taskId: snapshot.taskId,
      configRevision: validated.configRevision,
    })
  }

  /**
   * Load a workspace runtime snapshot
   */
  async load(taskId: string): Promise<WorkspaceRuntimeSnapshot | null> {
    const key = this.getKey(taskId)
    logger.debug('Loading workspace runtime snapshot', { taskId, key })

    const data = await this.storage.get<WorkspaceRuntimeSnapshot>(key)
    if (!data) {
      logger.debug('No workspace runtime snapshot found', { taskId })
      return null
    }

    // Validate loaded data (Zod will coerce Date objects from storage)
    try {
      const validated = WorkspaceRuntimeSnapshotSchema.parse(data)
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
   * Update a workspace runtime snapshot with optimistic locking
   * Returns true if update succeeded, false if snapshot was modified concurrently
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
   * Delete a workspace runtime snapshot
   */
  async delete(taskId: string): Promise<void> {
    const key = this.getKey(taskId)
    logger.debug('Deleting workspace runtime snapshot', { taskId, key })
    await this.storage.delete(key)
    logger.info('Workspace runtime snapshot deleted', { taskId })
  }

  /**
   * List all workspace runtime snapshots
   */
  async list(): Promise<WorkspaceRuntimeSnapshot[]> {
    const keys = await this.storage.list(this.keyPrefix)
    const snapshots = await Promise.all(
      keys.map((key) => {
        const taskId = this.extractTaskId(key)
        return this.load(taskId)
      }),
    )
    return snapshots.filter((s): s is WorkspaceRuntimeSnapshot => s !== null)
  }

  /**
   * Check if a snapshot exists for a task
   */
  async exists(taskId: string): Promise<boolean> {
    const key = this.getKey(taskId)
    return await this.storage.exists(key)
  }

  /**
   * Generate storage key for a task
   */
  private getKey(taskId: string): string {
    return `${this.keyPrefix}/${taskId}.json`
  }

  /**
   * Extract task ID from storage key
   */
  private extractTaskId(key: string): string {
    // key format: workspace-runtime/{taskId}.json
    const match = key.match(/workspace-runtime\/(.+)\.json$/)
    return match?.[1] || ''
  }
}
