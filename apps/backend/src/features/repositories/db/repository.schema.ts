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

  // Code execution fields (flattened from CodeExecution)
  setupCommands: text('setup_commands', { mode: 'json' }).$type<string[]>(),
  envVars: text('env_vars', { mode: 'json' }).$type<Record<string, string>>(),

  // Preview configuration fields (flattened from PreviewConfig)
  serverStartCommands: text('server_start_commands', { mode: 'json' }).$type<string[]>(),
  buildCommand: text('build_command'),
  buildOutputDirectory: text('build_output_directory').default('dist'),
  takeScreenshots: integer('take_screenshots', { mode: 'boolean' }),
  enableInspector: integer('enable_inspector', { mode: 'boolean' }).default(true),
  startupTimeoutSeconds: integer('startup_timeout_seconds').default(30),

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
 * Handles field name mapping (camelCase to snake_case) and flattening of nested objects.
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
    // Flatten CodeExecution fields
    setupCommands: repo.codeExecution?.setupCommands as string[] | undefined,
    envVars: repo.codeExecution?.envVars as Record<string, string> | undefined,
    // Flatten PreviewConfig fields
    serverStartCommands: repo.previewConfig?.serverStartCommands as string[] | undefined,
    buildCommand: repo.previewConfig?.buildCommand,
    buildOutputDirectory: repo.previewConfig?.buildOutputDirectory,
    takeScreenshots: repo.previewConfig?.takeScreenshots,
    enableInspector: repo.previewConfig?.enableInspector,
    startupTimeoutSeconds: repo.previewConfig?.startupTimeoutSeconds,
    createdAt: repo.createdAt,
    updatedAt: repo.updatedAt,
  }
}

/**
 * Reconstruct CodeExecution object from flattened database fields.
 */
function reconstructCodeExecution(row: RepositorySelect): CodeExecution {
  const codeExecution: CodeExecution = {}
  if (row.setupCommands) {
    codeExecution.setupCommands = row.setupCommands as string[]
  }
  if (row.envVars) {
    codeExecution.envVars = row.envVars as Record<string, string>
  }
  return codeExecution
}

/**
 * Reconstruct PreviewConfig object from flattened database fields.
 */
function reconstructPreviewConfig(row: RepositorySelect): PreviewConfig | undefined {
  const hasAnyPreviewField =
    row.serverStartCommands ||
    row.buildCommand ||
    row.buildOutputDirectory ||
    row.takeScreenshots !== null ||
    row.enableInspector !== null ||
    row.startupTimeoutSeconds !== null

  if (!hasAnyPreviewField) {
    return undefined
  }

  const previewConfig: PreviewConfig = {}
  if (row.serverStartCommands) {
    previewConfig.serverStartCommands = row.serverStartCommands as string[]
  }
  if (row.buildCommand) {
    previewConfig.buildCommand = row.buildCommand
  }
  if (row.buildOutputDirectory) {
    previewConfig.buildOutputDirectory = row.buildOutputDirectory
  }
  if (row.takeScreenshots !== null) {
    previewConfig.takeScreenshots = row.takeScreenshots ?? undefined
  }
  if (row.enableInspector !== null) {
    previewConfig.enableInspector = row.enableInspector ?? undefined
  }
  if (row.startupTimeoutSeconds !== null) {
    previewConfig.startupTimeoutSeconds = row.startupTimeoutSeconds ?? undefined
  }
  return previewConfig
}

/**
 * Convert database row to RepositoryConfig domain type.
 * Handles field name mapping (snake_case to camelCase) and reconstructing nested objects.
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
    codeExecution: reconstructCodeExecution(row),
    previewConfig: reconstructPreviewConfig(row),
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}
