import type { RepositoryConfig } from '@moatless/types'
import { and, eq, ilike, isNull, or } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import {
  assertVisibilityWritePermission,
  DatabaseSymbol,
  withVisibilityScope,
} from '../../../infrastructure/database/database'
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
          kind: insert.kind,
          description: insert.description,
          remoteUrl: insert.remoteUrl,
          provider: insert.provider,
          defaultBranch: insert.defaultBranch,
          deleted: insert.deleted,
          templateId: insert.templateId,
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
          enableInspector: insert.enableInspector,
          startupTimeoutSeconds: insert.startupTimeoutSeconds,
          autoReload: insert.autoReload,
          // Server configuration
          servers: insert.servers,
          // Plugin configuration
          plugins: insert.plugins,
          updatedAt: insert.updatedAt,
        },
      })
  }

  /**
   * Get a repository by ID.
   * Returns null for soft-deleted repositories.
   * Applies repository read access control (visibility + ownership).
   *
   * @param id - Repository identifier
   * @returns Repository configuration or null if not found, deleted, or not accessible
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

    // Filter out soft-deleted repositories
    if (row.deleted) {
      return null
    }

    return selectToRepository(row)
  }

  /**
   * Get a repository by ID without access control (for internal/system operations).
   * Use this for background tasks like event processing where there's no user session context.
   *
   * @param id - Repository identifier
   * @returns Repository configuration or null if not found
   */
  async getInternal(id: string): Promise<RepositoryConfig | null> {
    const rows = await this.db.drizzle
      .select()
      .from(repositories)
      .where(eq(repositories.id, id))
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
    // Get visibility scope condition
    const visibilityCondition = withVisibilityScope(repositories)

    // Filter: deleted = false OR deleted IS NULL (exclude soft-deleted)
    const notDeletedCondition = or(eq(repositories.deleted, false), isNull(repositories.deleted))

    // Combine conditions (visibility may be undefined in dev mode)
    const whereCondition = visibilityCondition
      ? and(visibilityCondition, notDeletedCondition)
      : notDeletedCondition

    const rows = await this.db.drizzle
      .select({ id: repositories.id })
      .from(repositories)
      .where(whereCondition)

    return rows.map((row) => row.id)
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
   * Get all repositories.
   * Applies repository read access control (visibility + ownership).
   *
   * @param includeDeleted - Whether to include soft-deleted repositories
   * @returns Array of all repository configurations
   */
  async getAll(includeDeleted = false): Promise<RepositoryConfig[]> {
    // Get visibility scope condition
    const visibilityCondition = withVisibilityScope(repositories)

    // Filter: deleted = false OR deleted IS NULL (exclude soft-deleted) unless includeDeleted
    const notDeletedCondition = or(eq(repositories.deleted, false), isNull(repositories.deleted))

    // Combine conditions based on includeDeleted flag
    let whereCondition: ReturnType<typeof and>
    if (includeDeleted) {
      whereCondition = visibilityCondition
    } else {
      whereCondition = visibilityCondition
        ? and(visibilityCondition, notDeletedCondition)
        : notDeletedCondition
    }

    const rows = await this.db.drizzle.select().from(repositories).where(whereCondition)

    return rows.map(selectToRepository)
  }

  /**
   * Find a repository by name and user ID.
   * Used for email integration: {username}.{repository}@inbox.moatless.ai
   * Does not apply visibility scope since we're matching by explicit user ID.
   *
   * @param name - The repository name
   * @param userId - The user ID who owns the repository
   * @returns Repository configuration or null if not found
   */
  async findByNameAndUser(name: string, userId: string): Promise<RepositoryConfig | null> {
    const rows = await this.db.drizzle
      .select()
      .from(repositories)
      .where(
        and(
          ilike(repositories.name, name),
          eq(repositories.createdBy, userId),
          or(eq(repositories.deleted, false), isNull(repositories.deleted)),
        ),
      )
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToRepository(row)
  }

  /**
   * Find a repository by remote URL.
   * Excludes soft-deleted repositories.
   * Does not apply visibility scope (matches by exact URL for seeding).
   *
   * @param remoteUrl - The remote URL to search for
   * @returns Repository configuration or null if not found
   */
  async findByRemoteUrl(remoteUrl: string): Promise<RepositoryConfig | null> {
    const rows = await this.db.drizzle
      .select()
      .from(repositories)
      .where(
        and(
          eq(repositories.remoteUrl, remoteUrl),
          or(eq(repositories.deleted, false), isNull(repositories.deleted)),
        ),
      )
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToRepository(row)
  }
}
