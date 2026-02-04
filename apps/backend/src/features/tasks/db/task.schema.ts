import type { Task } from '@moatless/types'
import { sql } from 'drizzle-orm'
import {
  boolean,
  doublePrecision,
  index,
  pgTable,
  serial,
  text,
  timestamp,
  uniqueIndex,
} from 'drizzle-orm/pg-core'

/**
 * Tasks table schema (PostgreSQL).
 * Stores core task metadata with flat fields only.
 * Computed fields (title, repositoryName) are derived at read time.
 */
export const tasks = pgTable(
  'tasks',
  {
    id: serial('id').primaryKey(),
    taskId: text('task_id').notNull(),
    shortId: text('short_id'), // Short alphanumeric ID for external references (e.g., email threading)
    createdAt: text('created_at').notNull(),
    lastMessageAt: text('last_message_at').notNull(),
    agentType: text('agent_type').notNull(),
    repositoryId: text('repository_id').notNull(),
    status: text('status').notNull(),
    totalCostUsd: doublePrecision('total_cost_usd').notNull(),
    siteUrl: text('site_url'), // Preview URL for the task
    templateId: text('template_id'),
    vibe: boolean('vibe'),
    name: text('name'),
    description: text('description'),
    siteId: text('site_id'),
    siteProvider: text('site_provider'),
    branch: text('branch'), // Branch to clone from (selected at task creation)
    createdBy: text('created_by'), // User ID who created this task (nullable for legacy tasks)
    visibility: text('visibility', { enum: ['private', 'public'] })
      .notNull()
      .default('private'),
    createdAtDb: timestamp('created_at_db').notNull().default(sql`now()`),
  },
  (table) => ({
    taskIdIdx: uniqueIndex('tasks_task_id_idx').on(table.taskId),
    shortIdIdx: uniqueIndex('tasks_short_id_idx').on(table.shortId),
    statusIdx: index('tasks_status_idx').on(table.status),
    repositoryIdx: index('tasks_repository_idx').on(table.repositoryId),
    lastMessageIdx: index('tasks_last_message_idx').on(table.lastMessageAt),
    createdByIdx: index('tasks_created_by_idx').on(table.createdBy),
    visibilityIdx: index('tasks_visibility_idx').on(table.visibility),
  }),
)

export type TaskInsert = typeof tasks.$inferInsert
export type TaskSelect = typeof tasks.$inferSelect

/**
 * Convert Task domain model to database insert format
 */
export function taskToInsert(task: Task): TaskInsert {
  return {
    taskId: task.taskId,
    shortId: task.shortId ?? null,
    createdAt: task.createdAt.toISOString(),
    lastMessageAt: task.lastMessageAt.toISOString(),
    agentType: task.agentType,
    repositoryId: task.repositoryId,
    status: task.status,
    totalCostUsd: task.totalCostUsd,
    siteUrl: task.siteUrl ?? null,
    templateId: task.templateId ?? null,
    vibe: task.vibe ?? null,
    name: task.name ?? null,
    description: task.description ?? null,
    siteId: task.siteId ?? null,
    siteProvider: task.siteProvider ?? null,
    branch: task.branch ?? null,
    createdBy: task.createdBy ?? null,
    visibility: task.visibility,
  }
}

/**
 * Convert database row to Task domain model
 */
export function selectToTask(taskRow: TaskSelect): Omit<Task, 'title' | 'repositoryName'> {
  return {
    taskId: taskRow.taskId,
    shortId: taskRow.shortId ?? undefined,
    createdAt: new Date(taskRow.createdAt),
    lastMessageAt: new Date(taskRow.lastMessageAt),
    agentType: taskRow.agentType,
    repositoryId: taskRow.repositoryId,
    status: taskRow.status as Task['status'],
    totalCostUsd: taskRow.totalCostUsd,
    siteUrl: taskRow.siteUrl ?? undefined,
    templateId: taskRow.templateId ?? undefined,
    vibe: taskRow.vibe ?? undefined,
    name: taskRow.name ?? undefined,
    description: taskRow.description ?? undefined,
    siteId: taskRow.siteId ?? undefined,
    siteProvider: taskRow.siteProvider ?? undefined,
    branch: taskRow.branch ?? undefined,
    createdBy: taskRow.createdBy ?? undefined,
    visibility: taskRow.visibility as 'private' | 'public',
  }
}
