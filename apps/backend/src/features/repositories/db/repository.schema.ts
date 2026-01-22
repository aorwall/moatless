import type { CodeExecution, PreviewConfig, RepositoryConfig } from '@moatless/types'
import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

/**
 * Repositories table schema.
 * Stores repository configurations for code execution and preview.
 *
 * Corresponds to the RepositoryConfig discriminated union in @moatless/types.
 * Supports two repository kinds: remote, template
 */
export const repositories = sqliteTable(
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
    templateId: text('template_id'), // Deprecated: local repos no longer supported

    // Remote/Template fields
    remoteUrl: text('remote_url'), // Required for remote/template
    provider: text('provider'), // 'github' | 'gitlab'
    defaultBranch: text('default_branch'), // Default branch from provider

    // Feature flags
    deleted: integer('deleted', { mode: 'boolean' }),

    // System prompt
    systemPrompt: text('system_prompt'),

    // Code execution fields (flattened from CodeExecution)
    setupCommands: text('setup_commands', { mode: 'json' }).$type<string[]>(),
    dockerImage: text('docker_image'),
    envVars: text('env_vars', { mode: 'json' }).$type<Record<string, string>>(),

    // Preview configuration fields (flattened from PreviewConfig)
    serverStartCommands: text('server_start_commands', { mode: 'json' }).$type<string[]>(),
    buildCommand: text('build_command'),
    buildOutputDirectory: text('build_output_directory').default('dist'),
    takeScreenshots: integer('take_screenshots', { mode: 'boolean' }),
    enableInspector: integer('enable_inspector', { mode: 'boolean' }).default(true),
    startupTimeoutSeconds: integer('startup_timeout_seconds').default(90),
    autoReload: integer('auto_reload', { mode: 'boolean' }),

    // Access control
    createdBy: text('created_by'), // User ID who created this repository (nullable for legacy/shared repos)
    visibility: text('visibility', { enum: ['private', 'public'] })
      .notNull()
      .default('private'), // 'private' = creator only, 'public' = all users can read

    // Timestamps stored as Unix epoch in milliseconds
    createdAt: integer('created_at', { mode: 'timestamp' }).notNull(),
    updatedAt: integer('updated_at', { mode: 'timestamp' }).notNull(),
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
    // Flatten PreviewConfig fields - use null instead of undefined to prevent DB defaults
    serverStartCommands: repo.previewConfig?.serverStartCommands as string[] | undefined,
    buildCommand: repo.previewConfig?.buildCommand ?? null,
    buildOutputDirectory: repo.previewConfig?.buildOutputDirectory ?? null,
    takeScreenshots: repo.previewConfig?.takeScreenshots ?? null,
    enableInspector: repo.previewConfig?.enableInspector ?? null,
    startupTimeoutSeconds: repo.previewConfig?.startupTimeoutSeconds ?? null,
    autoReload: repo.previewConfig?.autoReload ?? null,
    createdBy: repo.createdBy ?? null,
    visibility: repo.visibility ?? 'private',
    createdAt: repo.createdAt ?? new Date(),
    updatedAt: repo.updatedAt ?? new Date(),
  }

  // Handle discriminated union fields
  switch (repo.kind) {
    case 'remote':
      return {
        ...base,
        templateId: null,
        remoteUrl: repo.remote.url,
        provider: repo.remote.provider,
        defaultBranch: repo.remote.defaultBranch,
      }

    case 'template':
      return {
        ...base,
        templateId: null,
        remoteUrl: repo.remote.url,
        provider: repo.remote.provider,
        defaultBranch: repo.remote.defaultBranch,
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
 * Only includes fields that were explicitly set (non-null values).
 */
function reconstructPreviewConfig(row: RepositorySelect): PreviewConfig | undefined {
  // Check if any preview field has a non-null value
  const hasAnyPreviewField =
    row.serverStartCommands !== null ||
    row.buildCommand !== null ||
    row.buildOutputDirectory !== null ||
    row.takeScreenshots !== null ||
    row.enableInspector !== null ||
    row.startupTimeoutSeconds !== null ||
    row.autoReload !== null

  if (!hasAnyPreviewField) {
    return undefined
  }

  const previewConfig: PreviewConfig = {}

  // Only include fields that have non-null values (were explicitly set)
  if (row.serverStartCommands !== null) {
    previewConfig.serverStartCommands = row.serverStartCommands as string[]
  }
  if (row.buildCommand !== null) {
    previewConfig.buildCommand = row.buildCommand
  }
  if (row.buildOutputDirectory !== null) {
    previewConfig.buildOutputDirectory = row.buildOutputDirectory
  }
  if (row.takeScreenshots !== null) {
    previewConfig.takeScreenshots = row.takeScreenshots
  }
  if (row.enableInspector !== null) {
    previewConfig.enableInspector = row.enableInspector
  }
  if (row.startupTimeoutSeconds !== null) {
    previewConfig.startupTimeoutSeconds = row.startupTimeoutSeconds
  }
  if (row.autoReload !== null) {
    previewConfig.autoReload = row.autoReload
  }

  return previewConfig
}

/**
 * Convert database row to RepositoryConfig domain type.
 * Handles field name mapping (snake_case to camelCase) and reconstructing nested objects.
 * Reconstructs the proper discriminated union type based on the 'kind' field.
 */
export function selectToRepository(row: RepositorySelect): RepositoryConfig {
  // Coerce timestamps to Date objects - handles both INTEGER (epoch ms) and TEXT (ISO strings)
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
