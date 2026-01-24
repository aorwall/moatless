import type { Task, TaskIteration } from '@moatless/types'
import { sql } from 'drizzle-orm'
import {
  boolean,
  doublePrecision,
  index,
  integer,
  jsonb,
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
    numTurns: integer('num_turns').notNull(),
    totalCostUsd: doublePrecision('total_cost_usd').notNull(),
    workspacePath: text('workspace_path'),
    templateId: text('template_id'),
    vibe: boolean('vibe'),
    name: text('name'),
    description: text('description'),
    siteId: text('site_id'),
    siteProvider: text('site_provider'),
    gitCheckpointEnabled: boolean('git_checkpoint_enabled'),
    gitPushEnabled: boolean('git_push_enabled'),
    targetBranch: text('target_branch'), // Branch for git checkpoint pushes
    createdBy: text('created_by'), // User ID who created this task (nullable for legacy tasks)
    visibility: text('visibility', { enum: ['private', 'public'] })
      .notNull()
      .default('private'),
    background: boolean('background'),
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

/**
 * Task iterations table schema (PostgreSQL).
 * Stores iteration metadata with JSON columns for complex nested git data.
 * Related to tasks via task_id foreign key with cascade delete.
 */
export const taskIterations = pgTable(
  'task_iterations',
  {
    id: serial('id').primaryKey(),
    taskId: text('task_id')
      .notNull()
      .references(() => tasks.taskId, { onDelete: 'cascade' }),
    iterationNumber: integer('iteration_number').notNull(),
    startTime: text('start_time').notNull(),
    endTime: text('end_time'),
    status: text('status').notNull(),
    messageCount: integer('message_count').notNull(),
    costUsd: doublePrecision('cost_usd').notNull(),
    contextTokens: integer('context_tokens'),
    initialMessage: text('initial_message'), // TODO: Make sure this is saved as message and remove
    commandName: text('command_name'),
    result: text('result'),
    gitDiff: text('git_diff'),
    commitHash: text('commit_hash'),
    merged: boolean('merged'),
    agentSessionId: text('agent_session_id'),
    model: text('model'),
    agentMode: text('agent_mode'),
    gitStatus: jsonb('git_status').$type<TaskIteration['gitStatus'] | undefined>(),
    gitChanges: jsonb('git_changes').$type<TaskIteration['gitChanges'] | undefined>(),
    gitSyncStatus: jsonb('git_sync_status').$type<TaskIteration['gitSyncStatus'] | undefined>(),
    willNotResume: boolean('will_not_resume'),
    screenshotPath: text('screenshot_path'),
    thumbnailPath: text('thumbnail_path'),
    screenshotFileId: text('screenshot_file_id'),
    siteUrl: text('site_url'),
    checkpointCommitHash: text('checkpoint_commit_hash'),
    checkpointPushed: boolean('checkpoint_pushed'),
    background: boolean('background'),
    createdAt: timestamp('created_at').notNull().default(sql`now()`),
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
    shortId: task.shortId ?? null,
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
    targetBranch: task.targetBranch ?? null,
    createdBy: task.createdBy ?? null,
    visibility: task.visibility,
    background: task.background ?? null,
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
    background: iteration.background ?? null,
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
    background: row.background ?? undefined,
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
    shortId: taskRow.shortId ?? undefined,
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
    targetBranch: taskRow.targetBranch ?? undefined,
    createdBy: taskRow.createdBy ?? undefined,
    visibility: taskRow.visibility as 'private' | 'public',
    background: taskRow.background ?? undefined,
  }
}
