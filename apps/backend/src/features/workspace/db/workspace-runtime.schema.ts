import { integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'
import type {
  AgentState,
  ExecutionTarget,
  PreviewState,
  WorkspaceLock,
  WorkspaceRuntimeSnapshot,
} from '../types/workspace-runtime.types.js'

/**
 * Workspace runtimes table schema.
 * Stores workspace runtime state including execution target, agent, and preview state.
 *
 * Nested objects (ExecutionTarget, AgentState, PreviewState, WorkspaceLock[])
 * are stored as JSON text for atomic updates.
 */
export const workspaceRuntimes = sqliteTable('workspace_runtimes', {
  // Primary key - task ID
  taskId: text('task_id').primaryKey(),

  // Core fields
  repositoryId: text('repository_id').notNull(),
  configRevision: text('config_revision').notNull(),

  // Complex nested objects stored as JSON
  executionTarget: text('execution_target').notNull(),
  agent: text('agent'),
  preview: text('preview').notNull(),
  locks: text('locks'),

  // Timestamps stored as Unix epoch in milliseconds
  createdAt: integer('created_at', { mode: 'timestamp' }).notNull(),
  updatedAt: integer('updated_at', { mode: 'timestamp' }).notNull(),
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
    configRevision: snapshot.configRevision,
    executionTarget: JSON.stringify(snapshot.executionTarget),
    agent: snapshot.agent ? JSON.stringify(snapshot.agent) : null,
    preview: JSON.stringify(snapshot.preview),
    locks: snapshot.locks ? JSON.stringify(snapshot.locks) : null,
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
    configRevision: row.configRevision,
    executionTarget: JSON.parse(row.executionTarget) as ExecutionTarget,
    preview: JSON.parse(row.preview) as PreviewState,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }

  // Only assign optional fields if they exist
  if (row.agent !== null) {
    snapshot.agent = JSON.parse(row.agent) as AgentState
  }

  if (row.locks !== null) {
    snapshot.locks = JSON.parse(row.locks) as WorkspaceLock[]
  }

  return snapshot
}
