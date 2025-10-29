import type { CodeExecution, PreviewConfig, RepositoryConfig } from '@moatless/types'
import { integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

/**
 * Repositories table schema.
 * Stores repository configurations for code execution and preview.
 *
 * Corresponds to the RepositoryConfig interface in @moatless/types.
 */
export const repositories = sqliteTable('repositories', {
  // Primary key - repository UUID
  id: text('id').primaryKey(),

  // Basic repository information
  name: text('name').notNull(),
  description: text('description'),
  path: text('path').notNull(),
  remoteUrl: text('remote_url'),

  // Source and configuration
  sourceType: text('source_type'), // 'local' | 'github' | 'template'
  useGitWorktrees: integer('use_git_worktrees', { mode: 'boolean' }),
  cloneOnNewTask: integer('clone_on_new_task', { mode: 'boolean' }).notNull().default(false),
  originalBranch: text('original_branch'),

  // Feature flags
  deleted: integer('deleted', { mode: 'boolean' }),
  templateId: text('template_id'),
  vibe: integer('vibe', { mode: 'boolean' }),
  syncWithRepoConfig: integer('sync_with_repo_config', { mode: 'boolean' }),

  // System prompt
  systemPrompt: text('system_prompt'),

  // Complex nested objects stored as JSON
  codeExecution: text('code_execution', { mode: 'json' }).notNull().$type<CodeExecution>(),
  previewConfig: text('preview_config', { mode: 'json' }).$type<PreviewConfig>(),

  // Timestamps stored as Unix epoch in milliseconds
  createdAt: integer('created_at', { mode: 'timestamp' }).notNull(),
  updatedAt: integer('updated_at', { mode: 'timestamp' }).notNull(),
})

/**
 * Inferred TypeScript type for inserting a repository.
 * Generated from the schema definition.
 */
export type RepositoryInsert = typeof repositories.$inferInsert

/**
 * Inferred TypeScript type for selecting a repository from the database.
 * Generated from the schema definition.
 */
export type RepositorySelect = typeof repositories.$inferSelect

/**
 * Convert RepositoryConfig domain type to database insert type.
 * Handles field name mapping (camelCase to snake_case) and JSON serialization.
 */
export function repositoryToInsert(repo: RepositoryConfig): RepositoryInsert {
  return {
    id: repo.id,
    name: repo.name,
    description: repo.description,
    path: repo.path,
    remoteUrl: repo.remoteUrl,
    sourceType: repo.sourceType,
    useGitWorktrees: repo.useGitWorktrees,
    cloneOnNewTask: repo.cloneOnNewTask,
    originalBranch: repo.originalBranch,
    deleted: repo.deleted,
    templateId: repo.templateId,
    vibe: repo.vibe,
    syncWithRepoConfig: repo.syncWithRepoConfig,
    systemPrompt: repo.systemPrompt,
    codeExecution: repo.codeExecution as CodeExecution,
    previewConfig: repo.previewConfig as PreviewConfig | undefined,
    createdAt: repo.createdAt,
    updatedAt: repo.updatedAt,
  }
}

/**
 * Convert database row to RepositoryConfig domain type.
 * Handles field name mapping (snake_case to camelCase) and JSON parsing.
 */
export function selectToRepository(row: RepositorySelect): RepositoryConfig {
  return {
    id: row.id,
    name: row.name,
    description: row.description ?? undefined,
    path: row.path,
    remoteUrl: row.remoteUrl ?? undefined,
    sourceType: row.sourceType as 'local' | 'github' | 'template' | undefined,
    useGitWorktrees: row.useGitWorktrees ?? undefined,
    cloneOnNewTask: row.cloneOnNewTask,
    originalBranch: row.originalBranch ?? undefined,
    deleted: row.deleted ?? undefined,
    templateId: row.templateId ?? undefined,
    vibe: row.vibe ?? undefined,
    syncWithRepoConfig: row.syncWithRepoConfig ?? undefined,
    systemPrompt: row.systemPrompt ?? undefined,
    codeExecution: row.codeExecution as CodeExecution,
    previewConfig: row.previewConfig as PreviewConfig | undefined,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}
