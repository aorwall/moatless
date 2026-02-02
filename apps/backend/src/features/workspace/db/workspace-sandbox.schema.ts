import { jsonb, pgTable, text, timestamp } from 'drizzle-orm/pg-core'
import type { ExecutionTarget } from '../types/workspace-runtime.types'

/**
 * Workspace sandboxes table schema (PostgreSQL).
 * Single-table persistence layer for workspace state.
 * Stores execution target (needed for provisioner operations).
 * Agent/preview state is queried on-demand from the sandbox.
 */
export const workspaceSandboxes = pgTable('workspace_sandboxes', {
  /** Task ID — primary key, one sandbox per task */
  taskId: text('task_id').primaryKey(),

  /** Execution mode: 'docker' | 'kubernetes' */
  mode: text('mode').notNull(),

  /** Execution target configuration (Docker or K8s config) */
  executionTarget: jsonb('execution_target').notNull().$type<ExecutionTarget>(),

  /** Last user/agent interaction timestamp for inactivity tracking */
  lastActivityAt: timestamp('last_activity_at'),

  /** Persists error context after pod/container dies (cleared on recovery) */
  lastError: text('last_error'),

  /** Timestamps */
  createdAt: timestamp('created_at').notNull(),
  updatedAt: timestamp('updated_at').notNull(),
})

export type WorkspaceSandboxInsert = typeof workspaceSandboxes.$inferInsert
export type WorkspaceSandboxSelect = typeof workspaceSandboxes.$inferSelect

/**
 * Domain type for a sandbox record.
 * This is the sole persistence layer — agent/preview state is derived at runtime.
 */
export interface SandboxRecord {
  taskId: string
  mode: 'docker' | 'kubernetes'
  executionTarget: ExecutionTarget
  lastActivityAt: Date | null
  lastError: string | null
  createdAt: Date
  updatedAt: Date
}

/** Convert a DB row to a domain SandboxRecord */
export function selectToSandboxRecord(row: WorkspaceSandboxSelect): SandboxRecord {
  return {
    taskId: row.taskId,
    mode: row.mode as 'docker' | 'kubernetes',
    executionTarget: row.executionTarget,
    lastActivityAt: row.lastActivityAt,
    lastError: row.lastError,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}
