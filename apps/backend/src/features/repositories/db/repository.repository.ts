import type { RepositoryConfig } from '@moatless/types'
import { eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import {
  assertVisibilityWritePermission,
  DatabaseSymbol,
  withVisibilityScope,
} from '../../../infrastructure/database/database'
import { getSchemasFromEnv, type sqliteSchemas } from '../../../infrastructure/database/schemas'
import {
  repositoryToInsert as pgRepositoryToInsert,
  selectToRepository as pgSelectToRepository,
} from './pg/repository.schema'
import {
  repositoryToInsert as sqliteRepositoryToInsert,
  selectToRepository as sqliteSelectToRepository,
} from './repository.schema'

// Get correct schema based on DATABASE_PROVIDER
// Cast to SQLite schema type for TypeScript - runtime uses correct provider
const isPostgres = process.env['DATABASE_PROVIDER'] === 'postgres'
const schemas = getSchemasFromEnv()
const repositories = schemas.repositories as typeof sqliteSchemas.repositories
const repositoryToInsert = isPostgres ? pgRepositoryToInsert : sqliteRepositoryToInsert
const selectToRepository = isPostgres ? pgSelectToRepository : sqliteSelectToRepository

/**
 * Repository for repository configuration persistence using Drizzle ORM.
 * Provides type-safe database operations for repository entities.
 *
 * This replaces the legacy RepositoryStorage adapter, using a relational database
 * instead of JSON file storage.
 */
@injectable()
export class RepositoryRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Save a repository to the database.
   * Creates a new repository or updates an existing one (upsert).
   *
   * @param repository - Repository configuration to save
   */
  async save(repository: RepositoryConfig): Promise<void> {
    const insert = repositoryToInsert(repository)

    await this.db.drizzle
      .insert(repositories)
      .values(insert)
      .onConflictDoUpdate({
        target: repositories.id,
        set: {
          name: insert.name,
          kind: insert.kind,
          description: insert.description,
          path: insert.path,
          remoteUrl: insert.remoteUrl,
          sourceType: insert.sourceType,
          useGitWorktrees: insert.useGitWorktrees,
          cloneOnNewTask: insert.cloneOnNewTask,
          originalBranch: insert.originalBranch,
          deleted: insert.deleted,
          templateId: insert.templateId,
          vibe: insert.vibe,
          systemPrompt: insert.systemPrompt,
          visibility: insert.visibility,
          // Flattened CodeExecution fields
          setupCommands: insert.setupCommands,
          dockerImage: insert.dockerImage,
          envVars: insert.envVars,
          // Flattened PreviewConfig fields
          serverStartCommands: insert.serverStartCommands,
          buildCommand: insert.buildCommand,
          buildOutputDirectory: insert.buildOutputDirectory,
          takeScreenshots: insert.takeScreenshots,
          enableInspector: insert.enableInspector,
          startupTimeoutSeconds: insert.startupTimeoutSeconds,
          autoReload: insert.autoReload,
          updatedAt: insert.updatedAt,
        },
      })
  }

  /**
   * Get a repository by ID.
   * Returns the repository even if soft-deleted (direct ID lookups always succeed).
   * Soft-delete filtering only applies to list operations, not direct ID access.
   * Applies repository read access control (visibility + ownership).
   *
   * @param id - Repository identifier
   * @returns Repository configuration or null if not found or not accessible
   */
  async get(id: string): Promise<RepositoryConfig | null> {
    const rows = await this.db.drizzle
      .select()
      .from(repositories)
      .where(
        withVisibilityScope(repositories, {
          additionalConditions: [eq(repositories.id, id)],
        }),
      )
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToRepository(row)
  }

  /**
   * List all repository IDs.
   * Excludes soft-deleted repositories.
   * Applies repository read access control (visibility + ownership).
   *
   * @returns Array of repository IDs
   */
  async list(): Promise<string[]> {
    const rows = await this.db.drizzle
      .select({ id: repositories.id, deleted: repositories.deleted })
      .from(repositories)
      .where(withVisibilityScope(repositories))

    // Filter out soft-deleted repositories (deleted = true or 1)
    return rows.filter((row) => !row.deleted).map((row) => row.id)
  }

  /**
   * Delete a repository by ID (soft delete).
   * Updates the deleted flag to true.
   * Verifies user has write permission before deletion.
   *
   * @param id - Repository identifier
   * @throws {ForbiddenError} If user doesn't have write permission
   */
  async delete(id: string): Promise<void> {
    // Get repository first to verify access and write permission
    const repository = await this.get(id)
    if (!repository) {
      // Repository doesn't exist or user doesn't have read access
      return
    }

    // Verify write permission (throws ForbiddenError if not authorized)
    assertVisibilityWritePermission(repository, 'repository')

    await this.db.drizzle
      .update(repositories)
      .set({
        deleted: true,
        updatedAt: new Date(),
      })
      .where(eq(repositories.id, id))
  }

  /**
   * Check if a repository exists.
   * Returns false for soft-deleted repositories.
   *
   * @param id - Repository identifier
   * @returns True if repository exists and is not deleted
   */
  async exists(id: string): Promise<boolean> {
    const rows = await this.db.drizzle
      .select({ id: repositories.id, deleted: repositories.deleted })
      .from(repositories)
      .where(eq(repositories.id, id))
      .limit(1)

    const row = rows[0]
    return row !== undefined && !row.deleted
  }

  /**
   * Get all repositories (including soft-deleted).
   * Applies repository read access control (visibility + ownership).
   *
   * @param includeDeleted - Whether to include soft-deleted repositories
   * @returns Array of all repository configurations
   */
  async getAll(includeDeleted = false): Promise<RepositoryConfig[]> {
    const rows = await this.db.drizzle
      .select()
      .from(repositories)
      .where(withVisibilityScope(repositories))

    // Filter out soft-deleted repositories if not including them
    const filtered = includeDeleted ? rows : rows.filter((row) => !row.deleted)

    return filtered.map(selectToRepository)
  }
}
