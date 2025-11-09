import type {
  CodeExecution,
  LocalRepository,
  PreviewConfig,
  RemoteRepository,
  RepositoryConfig,
  TemplateRepository,
} from '@moatless/types'
import { integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

/**
 * Repositories table schema.
 * Stores repository configurations for code execution and preview.
 *
 * Corresponds to the RepositoryConfig discriminated union in @moatless/types.
 * Supports three repository kinds: local, remote, template
 */
export const repositories = sqliteTable('repositories', {
  // Primary key - repository UUID
  id: text('id').primaryKey(),

  // Discriminated union discriminator
  kind: text('kind').notNull(), // 'local' | 'remote' | 'template'

  // Basic repository information
  name: text('name').notNull(),
  description: text('description'),

  // Local fields (nullable for remote/template repos)
  path: text('path'), // Required for local, NULL for remote/template

  // Template source tracking (for local repos created from templates)
  templateId: text('template_id'), // References a template repository ID, NULL otherwise

  // Remote/Template fields (nullable for local repos)
  remoteUrl: text('remote_url'), // Required for remote/template, NULL for local
  provider: text('provider'), // 'github' | 'gitlab' | 'other' (nullable for local)
  defaultBranch: text('default_branch'), // Default branch from provider

  // Workspace modes (local/template only)
  useGitWorktrees: integer('use_git_worktrees', { mode: 'boolean' }),
  originalBranch: text('original_branch'),

  // Legacy fields (will be removed in future migration)
  sourceType: text('source_type'), // Deprecated: use 'kind' instead
  cloneOnNewTask: integer('clone_on_new_task', { mode: 'boolean' }),

  // Feature flags
  deleted: integer('deleted', { mode: 'boolean' }),
  vibe: integer('vibe', { mode: 'boolean' }), // Legacy template flag
  syncWithRepoConfig: integer('sync_with_repo_config', { mode: 'boolean' }),

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
  startupTimeoutSeconds: integer('startup_timeout_seconds').default(30),
  autoReload: integer('auto_reload', { mode: 'boolean' }),

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
  const base: RepositoryInsert = {
    id: repo.id,
    name: repo.name,
    description: repo.description,
    kind: repo.kind,
    deleted: repo.deleted,
    syncWithRepoConfig: repo.syncWithRepoConfig,
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
    createdAt: repo.createdAt,
    updatedAt: repo.updatedAt,
    // Legacy fields
    sourceType: null,
    cloneOnNewTask: null,
    vibe: null,
  }

  // Handle discriminated union fields
  switch (repo.kind) {
    case 'local':
      return {
        ...base,
        path: repo.path,
        useGitWorktrees: repo.useGitWorktrees,
        originalBranch: repo.originalBranch,
        templateId: null, // Local repos don't track template source (yet)
        remoteUrl: null,
        provider: null,
        defaultBranch: null,
      }

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
        path: null, // Templates don't have a local path
        templateId: null, // Templates ARE templates, they don't reference other templates
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
  const base = {
    id: row.id,
    name: row.name,
    description: row.description ?? undefined,
    deleted: row.deleted ?? undefined,
    syncWithRepoConfig: row.syncWithRepoConfig ?? undefined,
    systemPrompt: row.systemPrompt ?? undefined,
    codeExecution: reconstructCodeExecution(row),
    previewConfig: reconstructPreviewConfig(row),
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }

  // Reconstruct discriminated union based on kind
  const kind = row.kind as 'local' | 'remote' | 'template'

  switch (kind) {
    case 'local': {
      const local: LocalRepository = {
        ...base,
        kind: 'local',
        path: row.path!,
        useGitWorktrees: row.useGitWorktrees ?? false,
        originalBranch: row.originalBranch ?? undefined,
      }
      return local
    }

    case 'remote': {
      const remote: RemoteRepository = {
        ...base,
        kind: 'remote',
        remote: {
          url: row.remoteUrl!,
          provider: (row.provider as 'github' | 'gitlab' | 'other') || 'github',
          defaultBranch: row.defaultBranch ?? undefined,
        },
      }
      return remote
    }

    case 'template': {
      const template: TemplateRepository = {
        ...base,
        kind: 'template',
        remote: {
          url: row.remoteUrl!,
          provider: (row.provider as 'github' | 'gitlab' | 'other') || 'github',
          defaultBranch: row.defaultBranch ?? undefined,
        },
      }
      return template
    }

    default:
      throw new Error(`Unknown repository kind: ${kind}`)
  }
}
