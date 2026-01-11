import type { CodeExecution, PreviewConfig, RepositoryConfig } from '@moatless/types'
import { boolean, index, integer, jsonb, pgTable, text, timestamp } from 'drizzle-orm/pg-core'

/**
 * Repositories table schema (PostgreSQL).
 * Stores repository configurations for code execution and preview.
 *
 * Corresponds to the RepositoryConfig discriminated union in @moatless/types.
 * Supports two repository kinds: remote, template
 */
export const repositories = pgTable(
  'repositories',
  {
    // Primary key - repository UUID
    id: text('id').primaryKey(),

    // Discriminated union discriminator
    kind: text('kind').notNull(), // 'remote' | 'template'

    // Basic repository information
    name: text('name').notNull(),
    description: text('description'),

    // Legacy local fields (deprecated - kept for migration compatibility)
    path: text('path'), // Deprecated: local repos no longer supported
    templateId: text('template_id'), // Deprecated: local repos no longer supported
    useGitWorktrees: boolean('use_git_worktrees'), // Deprecated
    originalBranch: text('original_branch'), // Deprecated

    // Remote/Template fields
    remoteUrl: text('remote_url'), // Required for remote/template
    provider: text('provider'), // 'github' | 'gitlab'
    defaultBranch: text('default_branch'), // Default branch from provider

    // Legacy fields (will be removed in future migration)
    sourceType: text('source_type'), // Deprecated: use 'kind' instead
    cloneOnNewTask: boolean('clone_on_new_task'),

    // Feature flags
    deleted: boolean('deleted'),
    vibe: boolean('vibe'), // Legacy template flag

    // System prompt
    systemPrompt: text('system_prompt'),

    // Code execution fields (flattened from CodeExecution)
    setupCommands: jsonb('setup_commands').$type<string[]>(),
    dockerImage: text('docker_image'),
    envVars: jsonb('env_vars').$type<Record<string, string>>(),

    // Preview configuration fields (flattened from PreviewConfig)
    serverStartCommands: jsonb('server_start_commands').$type<string[]>(),
    buildCommand: text('build_command'),
    buildOutputDirectory: text('build_output_directory').default('dist'),
    takeScreenshots: boolean('take_screenshots'),
    enableInspector: boolean('enable_inspector').default(true),
    startupTimeoutSeconds: integer('startup_timeout_seconds').default(30),
    autoReload: boolean('auto_reload'),

    // Access control
    createdBy: text('created_by'), // User ID who created this repository (nullable for legacy/shared repos)
    visibility: text('visibility', { enum: ['private', 'public'] })
      .notNull()
      .default('private'), // 'private' = creator only, 'public' = all users can read

    // Timestamps
    createdAt: timestamp('created_at').notNull(),
    updatedAt: timestamp('updated_at').notNull(),
  },
  (table) => ({
    // Index for user-scoped queries
    createdByIdx: index('repositories_created_by_idx').on(table.createdBy),
    // Index for visibility-based queries
    visibilityIdx: index('repositories_visibility_idx').on(table.visibility),
  }),
)

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
  const base: RepositoryInsert = {
    id: repo.id,
    name: repo.name,
    description: repo.description,
    kind: repo.kind,
    deleted: repo.deleted,
    systemPrompt: repo.systemPrompt,
    // Flatten CodeExecution fields
    setupCommands: repo.codeExecution?.setupCommands as string[] | undefined,
    dockerImage: repo.codeExecution?.dockerImage,
    envVars: repo.codeExecution?.envVars as Record<string, string> | undefined,
    // Flatten PreviewConfig fields
    serverStartCommands: repo.previewConfig?.serverStartCommands as string[] | undefined,
    buildCommand: repo.previewConfig?.buildCommand,
    buildOutputDirectory: repo.previewConfig?.buildOutputDirectory,
    takeScreenshots: repo.previewConfig?.takeScreenshots,
    enableInspector: repo.previewConfig?.enableInspector,
    startupTimeoutSeconds: repo.previewConfig?.startupTimeoutSeconds,
    autoReload: repo.previewConfig?.autoReload,
    createdBy: repo.createdBy ?? null,
    visibility: repo.visibility ?? 'private',
    createdAt: repo.createdAt ?? new Date(),
    updatedAt: repo.updatedAt ?? new Date(),
    // Legacy fields
    sourceType: null,
    cloneOnNewTask: null,
    vibe: null,
  }

  // Handle discriminated union fields
  switch (repo.kind) {
    case 'remote':
      return {
        ...base,
        path: null,
        templateId: null,
        remoteUrl: repo.remote.url,
        provider: repo.remote.provider,
        defaultBranch: repo.remote.defaultBranch,
        useGitWorktrees: null,
        originalBranch: null,
      }

    case 'template':
      return {
        ...base,
        path: null,
        templateId: null,
        remoteUrl: repo.remote.url,
        provider: repo.remote.provider,
        defaultBranch: repo.remote.defaultBranch,
        useGitWorktrees: null,
        originalBranch: null,
      }
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
  if (row.dockerImage) {
    codeExecution.dockerImage = row.dockerImage
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
  // Only check meaningful fields that indicate intentional preview configuration
  // Exclude fields that have database defaults (buildOutputDirectory, enableInspector, startupTimeoutSeconds)
  const hasAnyPreviewField =
    row.serverStartCommands || row.buildCommand || row.takeScreenshots !== null

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
  // Always set takeScreenshots with proper default (false) to prevent undefined behavior
  previewConfig.takeScreenshots = row.takeScreenshots ?? false
  if (row.enableInspector !== null) {
    previewConfig.enableInspector = row.enableInspector ?? undefined
  }
  if (row.startupTimeoutSeconds !== null) {
    previewConfig.startupTimeoutSeconds = row.startupTimeoutSeconds ?? undefined
  }
  // Always set autoReload with proper default (false) to prevent undefined behavior
  previewConfig.autoReload = row.autoReload ?? false
  return previewConfig
}

/**
 * Convert database row to RepositoryConfig domain type.
 * Handles field name mapping (snake_case to camelCase) and reconstructing nested objects.
 * Reconstructs the proper discriminated union type based on the 'kind' field.
 */
export function selectToRepository(row: RepositorySelect): RepositoryConfig {
  // Coerce timestamps to Date objects - handles both timestamp and TEXT (ISO strings)
  // Drizzle may return Date objects or timestamp numbers depending on the mode
  const createdAt = new Date(row.createdAt)
  const updatedAt = new Date(row.updatedAt)

  const base = {
    id: row.id,
    name: row.name,
    description: row.description ?? undefined,
    deleted: row.deleted ?? undefined,
    systemPrompt: row.systemPrompt ?? undefined,
    codeExecution: reconstructCodeExecution(row),
    previewConfig: reconstructPreviewConfig(row),
    createdBy: row.createdBy ?? undefined,
    visibility: (row.visibility as 'private' | 'public') ?? 'private',
    createdAt,
    updatedAt,
  }

  // Reconstruct discriminated union based on kind
  const kind = row.kind as 'remote' | 'template'

  switch (kind) {
    case 'remote': {
      if (!row.provider) {
        throw new Error(
          `Provider is required for remote repository ${row.id}. Database integrity issue.`,
        )
      }
      return {
        ...base,
        kind: 'remote',
        remote: {
          url: row.remoteUrl!,
          provider: row.provider as 'github' | 'gitlab',
          defaultBranch: row.defaultBranch ?? undefined,
        },
      }
    }

    case 'template': {
      if (!row.provider) {
        throw new Error(
          `Provider is required for template repository ${row.id}. Database integrity issue.`,
        )
      }
      return {
        ...base,
        kind: 'template',
        remote: {
          url: row.remoteUrl!,
          provider: row.provider as 'github' | 'gitlab',
          defaultBranch: row.defaultBranch ?? undefined,
        },
      }
    }

    default:
      throw new Error(`Unknown repository kind: ${kind}`)
  }
}
