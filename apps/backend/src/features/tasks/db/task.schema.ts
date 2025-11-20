import type { Task, TaskIteration } from '@moatless/types'
import { sql } from 'drizzle-orm'
import { index, integer, real, sqliteTable, text, uniqueIndex } from 'drizzle-orm/sqlite-core'

/**
 * Tasks table schema.
 * Stores core task metadata with flat fields only.
 * Computed fields (title, repositoryName) are derived at read time.
 */
export const tasks = sqliteTable(
  'tasks',
  {
    id: integer('id').primaryKey({ autoIncrement: true }),
    taskId: text('task_id').notNull(),
    createdAt: text('created_at').notNull(),
    lastMessageAt: text('last_message_at').notNull(),
    agentType: text('agent_type').notNull(),
    repositoryId: text('repository_id').notNull(),
    status: text('status').notNull(),
    numTurns: integer('num_turns').notNull(),
    totalCostUsd: real('total_cost_usd').notNull(),
    workspacePath: text('workspace_path'),
    templateId: text('template_id'),
    vibe: integer('vibe', { mode: 'boolean' }),
    name: text('name'),
    description: text('description'),
    siteId: text('site_id'),
    siteProvider: text('site_provider'),
    gitCheckpointEnabled: integer('git_checkpoint_enabled', { mode: 'boolean' }),
    gitPushEnabled: integer('git_push_enabled', { mode: 'boolean' }),
    createdBy: text('created_by'), // User ID who created this task (nullable for legacy tasks)
    visibility: text('visibility', { enum: ['private', 'public'] })
      .notNull()
      .default('private'),
    createdAtDb: integer('created_at_db', { mode: 'timestamp' })
      .notNull()
      .default(sql`CURRENT_TIMESTAMP`),
  },
  (table) => ({
    taskIdIdx: uniqueIndex('tasks_task_id_idx').on(table.taskId),
    statusIdx: index('tasks_status_idx').on(table.status),
    repositoryIdx: index('tasks_repository_idx').on(table.repositoryId),
    lastMessageIdx: index('tasks_last_message_idx').on(table.lastMessageAt),
    createdByIdx: index('tasks_created_by_idx').on(table.createdBy),
    visibilityIdx: index('tasks_visibility_idx').on(table.visibility),
  }),
)

/**
 * Task iterations table schema.
 * Stores iteration metadata with JSON columns for complex nested git data.
 * Related to tasks via task_id foreign key with cascade delete.
 */
export const taskIterations = sqliteTable(
  'task_iterations',
  {
    id: integer('id').primaryKey({ autoIncrement: true }),
    taskId: text('task_id')
      .notNull()
      .references(() => tasks.taskId, { onDelete: 'cascade' }),
    iterationNumber: integer('iteration_number').notNull(),
    startTime: text('start_time').notNull(),
    endTime: text('end_time'),
    status: text('status').notNull(),
    messageCount: integer('message_count').notNull(),
    costUsd: real('cost_usd').notNull(),
    contextTokens: integer('context_tokens'),
    initialMessage: text('initial_message'),
    commandName: text('command_name'),
    result: text('result'),
    gitDiff: text('git_diff'),
    commitHash: text('commit_hash'),
    merged: integer('merged', { mode: 'boolean' }),
    agentSessionId: text('agent_session_id'),
    model: text('model'),
    agentMode: text('agent_mode'),
    gitStatus: text('git_status', { mode: 'json' }).$type<TaskIteration['gitStatus'] | undefined>(),
    gitChanges: text('git_changes', { mode: 'json' }).$type<
      TaskIteration['gitChanges'] | undefined
    >(),
    gitSyncStatus: text('git_sync_status', { mode: 'json' }).$type<
      TaskIteration['gitSyncStatus'] | undefined
    >(),
    willNotResume: integer('will_not_resume', { mode: 'boolean' }),
    screenshotPath: text('screenshot_path'),
    thumbnailPath: text('thumbnail_path'),
    screenshotFileId: text('screenshot_file_id'),
    siteUrl: text('site_url'),
    checkpointCommitHash: text('checkpoint_commit_hash'),
    checkpointPushed: integer('checkpoint_pushed', { mode: 'boolean' }),
    createdAt: integer('created_at', { mode: 'timestamp' })
      .notNull()
      .default(sql`CURRENT_TIMESTAMP`),
  },
  (table) => ({
    taskIterationIdx: index('task_iterations_task_iteration_idx').on(
      table.taskId,
      table.iterationNumber,
    ),
    statusIdx: index('task_iterations_status_idx').on(table.status),
  }),
)

export type TaskInsert = typeof tasks.$inferInsert
export type TaskSelect = typeof tasks.$inferSelect
export type TaskIterationInsert = typeof taskIterations.$inferInsert
export type TaskIterationSelect = typeof taskIterations.$inferSelect

/**
 * Convert Task domain model to database insert format
 */
export function taskToInsert(task: Task): TaskInsert {
  return {
    taskId: task.taskId,
    createdAt: task.createdAt.toISOString(),
    lastMessageAt: task.lastMessageAt.toISOString(),
    agentType: task.agentType,
    repositoryId: task.repositoryId,
    status: task.status,
    numTurns: task.numTurns,
    totalCostUsd: task.totalCostUsd,
    workspacePath: task.workspacePath ?? null,
    templateId: task.templateId ?? null,
    vibe: task.vibe ?? null,
    name: task.name ?? null,
    description: task.description ?? null,
    siteId: task.siteId ?? null,
    siteProvider: task.siteProvider ?? null,
    gitCheckpointEnabled: task.gitCheckpointEnabled ?? null,
    gitPushEnabled: task.gitPushEnabled ?? null,
    createdBy: task.createdBy ?? null,
    visibility: task.visibility,
  }
}

/**
 * Convert TaskIteration to database insert format
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Data mapping function with many optional fields
export function iterationToInsert(taskId: string, iteration: TaskIteration): TaskIterationInsert {
  return {
    taskId,
    iterationNumber: iteration.iterationNumber,
    startTime: iteration.startTime.toISOString(),
    endTime: iteration.endTime?.toISOString() ?? null,
    status: iteration.status,
    messageCount: iteration.messageCount,
    costUsd: iteration.costUsd,
    contextTokens: iteration.contextTokens ?? null,
    initialMessage: iteration.initialMessage ?? null,
    commandName: iteration.commandName ?? null,
    result: iteration.result ?? null,
    gitDiff: iteration.gitDiff ?? null,
    commitHash: iteration.commitHash ?? null,
    merged: iteration.merged ?? null,
    agentSessionId: iteration.agentSessionId ?? null,
    model: iteration.model ?? null,
    agentMode: iteration.agentMode ?? null,
    gitStatus: iteration.gitStatus ?? null,
    gitChanges: iteration.gitChanges ?? null,
    gitSyncStatus: iteration.gitSyncStatus ?? null,
    willNotResume: iteration.willNotResume ?? null,
    screenshotFileId: iteration.screenshotFileId ?? null,
    siteUrl: iteration.siteUrl ?? null,
    checkpointCommitHash: iteration.checkpointCommitHash ?? null,
    checkpointPushed: iteration.checkpointPushed ?? null,
  }
}

/**
 * Convert database row to TaskIteration domain model
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Data mapping function with many optional fields
export function selectToIteration(row: TaskIterationSelect): TaskIteration {
  return {
    iterationNumber: row.iterationNumber,
    startTime: new Date(row.startTime),
    endTime: row.endTime ? new Date(row.endTime) : undefined,
    status: row.status as TaskIteration['status'],
    messageCount: row.messageCount,
    costUsd: row.costUsd,
    contextTokens: row.contextTokens ?? undefined,
    initialMessage: row.initialMessage ?? undefined,
    commandName: row.commandName ?? undefined,
    result: row.result ?? undefined,
    gitDiff: row.gitDiff ?? undefined,
    commitHash: row.commitHash ?? undefined,
    merged: row.merged ?? undefined,
    agentSessionId: row.agentSessionId ?? undefined,
    model: row.model ?? undefined,
    agentMode: (row.agentMode as TaskIteration['agentMode']) ?? undefined,
    gitStatus: row.gitStatus ?? undefined,
    gitChanges: row.gitChanges ?? undefined,
    gitSyncStatus: row.gitSyncStatus ?? undefined,
    willNotResume: row.willNotResume ?? undefined,
    screenshotFileId: row.screenshotFileId ?? undefined,
    siteUrl: row.siteUrl ?? undefined,
    checkpointCommitHash: row.checkpointCommitHash ?? undefined,
    checkpointPushed: row.checkpointPushed ?? undefined,
  }
}

/**
 * Convert database rows to Task domain model
 */
export function selectToTask(
  taskRow: TaskSelect,
  iterationRows: TaskIterationSelect[],
): Omit<Task, 'title' | 'repositoryName'> {
  return {
    taskId: taskRow.taskId,
    createdAt: new Date(taskRow.createdAt),
    lastMessageAt: new Date(taskRow.lastMessageAt),
    agentType: taskRow.agentType,
    repositoryId: taskRow.repositoryId,
    status: taskRow.status as Task['status'],
    numTurns: taskRow.numTurns,
    totalCostUsd: taskRow.totalCostUsd,
    iterations: iterationRows.map(selectToIteration),
    workspacePath: taskRow.workspacePath ?? undefined,
    templateId: taskRow.templateId ?? undefined,
    vibe: taskRow.vibe ?? undefined,
    name: taskRow.name ?? undefined,
    description: taskRow.description ?? undefined,
    siteId: taskRow.siteId ?? undefined,
    siteProvider: taskRow.siteProvider ?? undefined,
    gitCheckpointEnabled: taskRow.gitCheckpointEnabled ?? undefined,
    gitPushEnabled: taskRow.gitPushEnabled ?? undefined,
    createdBy: taskRow.createdBy ?? undefined,
    visibility: taskRow.visibility as 'private' | 'public',
  }
}
