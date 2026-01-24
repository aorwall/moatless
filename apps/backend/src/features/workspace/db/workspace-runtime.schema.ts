import { pgTable, text, timestamp } from 'drizzle-orm/pg-core'
import type {
  AgentState,
  ExecutionTarget,
  PreviewState,
  WorkspaceLock,
  WorkspaceRuntimeSnapshot,
} from '../types/workspace-runtime.types'

/**
 * Workspace runtimes table schema (PostgreSQL).
 * Stores workspace runtime state including execution target, agent, and preview state.
 *
 * Nested objects (ExecutionTarget, AgentState, PreviewState, WorkspaceLock[])
 * are stored as JSON text for atomic updates.
 */
export const workspaceRuntimes = pgTable('workspace_runtimes', {
  // Primary key - task ID
  taskId: text('task_id').primaryKey(),

  // Core fields
  repositoryId: text('repository_id').notNull(),
  // Note: TypeScript property renamed to snapshotRevision for clarity, but DB column
  // remains 'config_revision' for backwards compatibility with existing databases
  snapshotRevision: text('config_revision').notNull(),

  // Complex nested objects stored as JSON text
  executionTarget: text('execution_target').notNull(),
  agent: text('agent'),
  preview: text('preview').notNull(),
  locks: text('locks'),

  // Activity tracking for inactivity timeout
  lastActivityAt: timestamp('last_activity_at'),

  // Timestamps
  createdAt: timestamp('created_at').notNull(),
  updatedAt: timestamp('updated_at').notNull(),
})

/**
 * Inferred TypeScript type for inserting a workspace runtime record.
 */
export type WorkspaceRuntimeInsert = typeof workspaceRuntimes.$inferInsert

/**
 * Inferred TypeScript type for selecting a workspace runtime record.
 */
export type WorkspaceRuntimeSelect = typeof workspaceRuntimes.$inferSelect

/**
 * Convert WorkspaceRuntimeSnapshot domain type to database insert type.
 * Serializes complex nested objects to JSON.
 */
export function snapshotToInsert(snapshot: WorkspaceRuntimeSnapshot): WorkspaceRuntimeInsert {
  return {
    taskId: snapshot.taskId,
    repositoryId: snapshot.repositoryId,
    snapshotRevision: snapshot.snapshotRevision,
    executionTarget: JSON.stringify(snapshot.executionTarget),
    agent: snapshot.agent ? JSON.stringify(snapshot.agent) : null,
    preview: JSON.stringify(snapshot.preview),
    locks: snapshot.locks ? JSON.stringify(snapshot.locks) : null,
    lastActivityAt: snapshot.lastActivityAt ?? null,
    createdAt: snapshot.createdAt,
    updatedAt: snapshot.updatedAt,
  }
}

/**
 * Convert database row to WorkspaceRuntimeSnapshot domain type.
 * Deserializes JSON fields back to typed objects.
 */
export function selectToSnapshot(row: WorkspaceRuntimeSelect): WorkspaceRuntimeSnapshot {
  const snapshot: WorkspaceRuntimeSnapshot = {
    taskId: row.taskId,
    repositoryId: row.repositoryId,
    snapshotRevision: row.snapshotRevision,
    executionTarget: JSON.parse(row.executionTarget) as ExecutionTarget,
    preview: JSON.parse(row.preview) as PreviewState,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }

  if (row.lastActivityAt !== null) {
    snapshot.lastActivityAt = row.lastActivityAt
  }

  // Only assign optional fields if they exist
  if (row.agent !== null) {
    snapshot.agent = JSON.parse(row.agent) as AgentState
  }

  if (row.locks !== null) {
    const parsedLocks = JSON.parse(row.locks) as Array<{
      type: WorkspaceLock['type']
      owner: string
      acquiredAt: string | Date
    }>
    // Convert acquiredAt strings back to Date objects
    snapshot.locks = parsedLocks.map((lock) => ({
      ...lock,
      acquiredAt: typeof lock.acquiredAt === 'string' ? new Date(lock.acquiredAt) : lock.acquiredAt,
    }))
  }

  return snapshot
}
