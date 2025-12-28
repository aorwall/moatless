/**
 * Schema loader for multi-dialect support.
 *
 * This module provides functions to load the appropriate schema definitions
 * based on the active database provider (SQLite or PostgreSQL).
 */

// SQLite schemas
import { commands as sqliteCommands } from '../../../features/agents/db/command.schema'
// PostgreSQL schemas
import { commands as pgCommands } from '../../../features/agents/db/pg/command.schema'
import { apiKeys as sqliteApiKeys } from '../../../features/auth/db/api-key.schema'
import { oauthStates as sqliteOauthStates } from '../../../features/auth/db/oauth-state.schema'
import { apiKeys as pgApiKeys } from '../../../features/auth/db/pg/api-key.schema'
import { oauthStates as pgOauthStates } from '../../../features/auth/db/pg/oauth-state.schema'
import { providerTokens as pgProviderTokens } from '../../../features/auth/db/pg/provider-token.schema'
import { sessions as pgSessions } from '../../../features/auth/db/pg/session.schema'
import { users as pgUsers } from '../../../features/auth/db/pg/user.schema'
import { providerTokens as sqliteProviderTokens } from '../../../features/auth/db/provider-token.schema'
import { sessions as sqliteSessions } from '../../../features/auth/db/session.schema'
import { users as sqliteUsers } from '../../../features/auth/db/user.schema'
import { repositories as pgRepositories } from '../../../features/repositories/db/pg/repository.schema'
import { repositories as sqliteRepositories } from '../../../features/repositories/db/repository.schema'
import {
  taskIterations as pgTaskIterations,
  tasks as pgTasks,
} from '../../../features/tasks/db/pg/task.schema'
import {
  taskMessageContexts as pgTaskMessageContexts,
  taskMessages as pgTaskMessages,
} from '../../../features/tasks/db/pg/task-message.schema'
import {
  taskIterations as sqliteTaskIterations,
  tasks as sqliteTasks,
} from '../../../features/tasks/db/task.schema'
import {
  taskMessageContexts as sqliteTaskMessageContexts,
  taskMessages as sqliteTaskMessages,
} from '../../../features/tasks/db/task-message.schema'
import { workspaceRuntimes as pgWorkspaceRuntimes } from '../../../features/workspace/db/pg/workspace-runtime.schema'
import { workspaceRuntimes as sqliteWorkspaceRuntimes } from '../../../features/workspace/db/workspace-runtime.schema'
import type { DatabaseProvider } from '../types'

/**
 * Schema collection type for type-safe access to all tables.
 */
export interface Schemas {
  users: typeof sqliteUsers | typeof pgUsers
  sessions: typeof sqliteSessions | typeof pgSessions
  apiKeys: typeof sqliteApiKeys | typeof pgApiKeys
  oauthStates: typeof sqliteOauthStates | typeof pgOauthStates
  providerTokens: typeof sqliteProviderTokens | typeof pgProviderTokens
  tasks: typeof sqliteTasks | typeof pgTasks
  taskIterations: typeof sqliteTaskIterations | typeof pgTaskIterations
  taskMessages: typeof sqliteTaskMessages | typeof pgTaskMessages
  taskMessageContexts: typeof sqliteTaskMessageContexts | typeof pgTaskMessageContexts
  repositories: typeof sqliteRepositories | typeof pgRepositories
  workspaceRuntimes: typeof sqliteWorkspaceRuntimes | typeof pgWorkspaceRuntimes
  commands: typeof sqliteCommands | typeof pgCommands
}

/**
 * SQLite schema collection.
 */
export const sqliteSchemas = {
  users: sqliteUsers,
  sessions: sqliteSessions,
  apiKeys: sqliteApiKeys,
  oauthStates: sqliteOauthStates,
  providerTokens: sqliteProviderTokens,
  tasks: sqliteTasks,
  taskIterations: sqliteTaskIterations,
  taskMessages: sqliteTaskMessages,
  taskMessageContexts: sqliteTaskMessageContexts,
  repositories: sqliteRepositories,
  workspaceRuntimes: sqliteWorkspaceRuntimes,
  commands: sqliteCommands,
} as const

/**
 * PostgreSQL schema collection.
 */
export const pgSchemas = {
  users: pgUsers,
  sessions: pgSessions,
  apiKeys: pgApiKeys,
  oauthStates: pgOauthStates,
  providerTokens: pgProviderTokens,
  tasks: pgTasks,
  taskIterations: pgTaskIterations,
  taskMessages: pgTaskMessages,
  taskMessageContexts: pgTaskMessageContexts,
  repositories: pgRepositories,
  workspaceRuntimes: pgWorkspaceRuntimes,
  commands: pgCommands,
} as const

/**
 * Get schemas for the specified database provider.
 *
 * @param provider - The database provider ('sqlite' or 'postgres')
 * @returns The appropriate schema collection
 */
export function getSchemas(provider: DatabaseProvider): typeof sqliteSchemas | typeof pgSchemas {
  return provider === 'postgres' ? pgSchemas : sqliteSchemas
}

/**
 * Get schemas based on environment variable.
 * Reads DATABASE_PROVIDER from environment, defaults to 'sqlite'.
 *
 * @returns The appropriate schema collection
 */
export function getSchemasFromEnv(): typeof sqliteSchemas | typeof pgSchemas {
  const provider = (process.env['DATABASE_PROVIDER'] || 'sqlite') as DatabaseProvider
  return getSchemas(provider)
}
