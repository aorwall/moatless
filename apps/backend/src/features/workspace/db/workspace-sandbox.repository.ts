import { eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { logger } from '../../../utils/logger'
import type { ExecutionTarget } from '../types/workspace-runtime.types'
import {
  type SandboxRecord,
  selectToSandboxRecord,
  workspaceSandboxes,
} from './workspace-sandbox.schema'

/**
 * Repository for workspace sandbox persistence.
 * Sole persistence layer for workspace state (replaces WorkspaceRuntimeRepository).
 * Stores execution target + coordination data.
 * Agent/preview state is queried on-demand from sandbox.
 */
@injectable()
export class WorkspaceSandboxRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Register a new sandbox (upsert) with execution target.
   */
  async register(
    taskId: string,
    mode: 'docker' | 'kubernetes',
    executionTarget: ExecutionTarget,
  ): Promise<void> {
    const now = new Date()
    await this.db.drizzle
      .insert(workspaceSandboxes)
      .values({
        taskId,
        mode,
        executionTarget,
        lastActivityAt: now,
        createdAt: now,
        updatedAt: now,
      })
      .onConflictDoUpdate({
        target: workspaceSandboxes.taskId,
        set: {
          mode,
          executionTarget,
          lastActivityAt: now,
          lastError: null,
          updatedAt: now,
        },
      })
    logger.debug({ taskId, mode }, 'Sandbox registered')
  }

  /**
   * Unregister (delete) a sandbox record.
   */
  async unregister(taskId: string): Promise<void> {
    await this.db.drizzle.delete(workspaceSandboxes).where(eq(workspaceSandboxes.taskId, taskId))
    logger.debug({ taskId }, 'Sandbox unregistered')
  }

  /**
   * Load a sandbox record by task ID.
   */
  async load(taskId: string): Promise<SandboxRecord | null> {
    const rows = await this.db.drizzle
      .select()
      .from(workspaceSandboxes)
      .where(eq(workspaceSandboxes.taskId, taskId))
      .limit(1)

    const row = rows[0]
    return row ? selectToSandboxRecord(row) : null
  }

  /**
   * Check if a sandbox exists.
   */
  async exists(taskId: string): Promise<boolean> {
    const rows = await this.db.drizzle
      .select({ taskId: workspaceSandboxes.taskId })
      .from(workspaceSandboxes)
      .where(eq(workspaceSandboxes.taskId, taskId))
      .limit(1)
    return rows.length > 0
  }

  /**
   * List all sandbox records (for inactivity checking, health collection).
   */
  async list(): Promise<SandboxRecord[]> {
    const rows = await this.db.drizzle.select().from(workspaceSandboxes)
    return rows.map(selectToSandboxRecord)
  }

  /**
   * Lightweight update of lastActivityAt timestamp.
   */
  async touchActivity(taskId: string, timestamp: Date): Promise<void> {
    await this.db.drizzle
      .update(workspaceSandboxes)
      .set({ lastActivityAt: timestamp, updatedAt: timestamp })
      .where(eq(workspaceSandboxes.taskId, taskId))
  }

  /**
   * Update the execution target for a sandbox.
   */
  async updateExecutionTarget(taskId: string, executionTarget: ExecutionTarget): Promise<void> {
    const now = new Date()
    await this.db.drizzle
      .update(workspaceSandboxes)
      .set({ executionTarget, mode: executionTarget.mode, updatedAt: now })
      .where(eq(workspaceSandboxes.taskId, taskId))
  }

  /**
   * Update execution target status field.
   */
  async updateStatus(
    taskId: string,
    updater: (current: ExecutionTarget) => ExecutionTarget,
  ): Promise<void> {
    const record = await this.load(taskId)
    if (!record) {
      return
    }
    const updated = updater(record.executionTarget)
    await this.updateExecutionTarget(taskId, updated)
  }

  /**
   * Set error on a sandbox record.
   */
  async setError(taskId: string, error: string): Promise<void> {
    const now = new Date()
    await this.db.drizzle
      .update(workspaceSandboxes)
      .set({ lastError: error, updatedAt: now })
      .where(eq(workspaceSandboxes.taskId, taskId))
  }

  /**
   * Clear error on a sandbox record.
   */
  async clearError(taskId: string): Promise<void> {
    const now = new Date()
    await this.db.drizzle
      .update(workspaceSandboxes)
      .set({ lastError: null, updatedAt: now })
      .where(eq(workspaceSandboxes.taskId, taskId))
  }
}
