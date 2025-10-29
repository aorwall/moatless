import type { RepositoryConfig } from '@moatless/types'
import { eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { repositories, repositoryToInsert, selectToRepository } from './repository.schema'

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
          syncWithRepoConfig: insert.syncWithRepoConfig,
          systemPrompt: insert.systemPrompt,
          codeExecution: insert.codeExecution,
          previewConfig: insert.previewConfig,
          updatedAt: insert.updatedAt,
        },
      })
  }

  /**
   * Get a repository by ID.
   * Returns null for repositories that are soft-deleted or not found.
   *
   * @param id - Repository identifier
   * @returns Repository configuration or null if not found/deleted
   */
  async get(id: string): Promise<RepositoryConfig | null> {
    const rows = await this.db.drizzle
      .select()
      .from(repositories)
      .where(eq(repositories.id, id))
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    // Filter out soft-deleted repositories
    if (row.deleted) {
      return null
    }

    return selectToRepository(row)
  }

  /**
   * List all repository IDs.
   * Excludes soft-deleted repositories.
   *
   * @returns Array of repository IDs
   */
  async list(): Promise<string[]> {
    const rows = await this.db.drizzle
      .select({ id: repositories.id, deleted: repositories.deleted })
      .from(repositories)

    // Filter out soft-deleted repositories (deleted = true or 1)
    return rows.filter((row) => !row.deleted).map((row) => row.id)
  }

  /**
   * Delete a repository by ID (soft delete).
   * Updates the deleted flag to true.
   *
   * @param id - Repository identifier
   */
  async delete(id: string): Promise<void> {
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
   * Useful for admin/migration purposes.
   *
   * @param includeDeleted - Whether to include soft-deleted repositories
   * @returns Array of all repository configurations
   */
  async getAll(includeDeleted = false): Promise<RepositoryConfig[]> {
    const rows = await this.db.drizzle.select().from(repositories)

    // Filter out soft-deleted repositories if not including them
    const filtered = includeDeleted ? rows : rows.filter((row) => !row.deleted)

    return filtered.map(selectToRepository)
  }
}
