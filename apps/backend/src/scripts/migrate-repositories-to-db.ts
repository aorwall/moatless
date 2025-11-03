#!/usr/bin/env bun
/**
 * One-time migration script to move repository data from JSON files to database.
 *
 * Usage:
 *   bun run scripts/migrate-repositories-to-db.ts
 *   or
 *   bun run scripts/migrate-repositories-to-db.ts --dry-run
 */

import { join } from 'node:path'
import type { RepositoryConfig } from '@moatless/types'
import { RepositoryRepository } from '../src/features/repositories/db/repository.repository'
import { createDrizzleDatabase } from '../src/infrastructure/database/drizzle'
import { FileStorage } from '../src/storage/file-storage'
import { getDataDir } from '../src/utils/data-dir'
import { logger } from '../src/utils/logger'

async function migrateRepositories(dryRun = false) {
  const dataDir = getDataDir()

  // Initialize database
  const dbPath = join(dataDir, 'moatless.db')
  const database = createDrizzleDatabase(dbPath, logger)
  await database.migrate()

  // Initialize repository
  const repository = new RepositoryRepository(database)

  // Initialize file storage
  const storage = new FileStorage(dataDir)

  try {
    // List all repository IDs from JSON storage
    const repoIds = await storage.list('repositories/')
    logger.info(`Found ${repoIds.length} repositories in JSON storage`)

    if (repoIds.length === 0) {
      logger.info('No repositories to migrate')
      return
    }

    let migratedCount = 0
    let skippedCount = 0
    let errorCount = 0

    for (const key of repoIds) {
      const repoId = key.replace('repositories/', '')

      try {
        // Get repository from JSON storage
        const repo = await storage.get<RepositoryConfig>(key)

        if (!repo) {
          logger.warn(`Repository ${repoId} not found in storage, skipping`)
          skippedCount++
          continue
        }

        // Check if already exists in database
        const existsInDb = await repository.exists(repoId)

        if (existsInDb) {
          logger.info(`Repository ${repoId} already exists in database, skipping`)
          skippedCount++
          continue
        }

        if (dryRun) {
          logger.info(`[DRY RUN] Would migrate repository ${repoId}: ${repo.name}`)
          migratedCount++
        } else {
          // Save to database
          await repository.save(repo)
          logger.info(`✓ Migrated repository ${repoId}: ${repo.name}`)
          migratedCount++
        }
      } catch (error) {
        logger.error(`✗ Failed to migrate repository ${repoId}`, error as Error)
        errorCount++
      }
    }

    logger.info('\n=== Migration Summary ===')
    logger.info(`Total repositories: ${repoIds.length}`)
    logger.info(`Migrated: ${migratedCount}`)
    logger.info(`Skipped: ${skippedCount}`)
    logger.info(`Errors: ${errorCount}`)

    if (dryRun) {
      logger.info('\n*** DRY RUN MODE - No changes were made ***')
    }
  } finally {
    await database.close()
  }
}

// Parse command line arguments
const args = process.argv.slice(2)
const dryRun = args.includes('--dry-run')

if (dryRun) {
  logger.info('Running in DRY RUN mode - no changes will be made\n')
}

migrateRepositories(dryRun)
  .then(() => {
    logger.info('\nMigration completed successfully')
    process.exit(0)
  })
  .catch((error) => {
    logger.error('Migration failed', error as Error)
    process.exit(1)
  })
