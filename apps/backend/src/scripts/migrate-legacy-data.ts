#!/usr/bin/env bun
/**
 * Unified migration script to move all legacy data formats to the database.
 *
 * Migrates:
 * - Repositories: JSON files → database
 * - Tasks: JSON files → database
 * - Messages: JSONL files → database
 *
 * Usage:
 *   bun run scripts/migrate-legacy-data.ts
 *   bun run scripts/migrate-legacy-data.ts --dry-run
 *   bun run scripts/migrate-legacy-data.ts --delete-source
 *   bun run scripts/migrate-legacy-data.ts --dry-run --delete-source
 */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { AgentMessage, RepositoryConfig, Task } from '@moatless/types'
import { RepositoryRepository } from '../features/repositories/db/repository.repository'
import { TaskRepository } from '../features/tasks/db/task.repository'
import { TaskMessageRepository } from '../features/tasks/db/task-message.repository'
import type { Database } from '../infrastructure/database/database'
import { createDrizzleDatabase } from '../infrastructure/database/drizzle'
import { FileStorage } from '../storage/file-storage'
import { getDataDir } from '../utils/data-dir'
import { logger } from '../utils/logger'

interface MigrationOptions {
  dryRun: boolean
  deleteSource: boolean
}

interface MigrationStats {
  repositories: { total: number; migrated: number; skipped: number; errors: number }
  tasks: { total: number; migrated: number; iterations: number; skipped: number; errors: number }
  messages: {
    total: number
    migratedTasks: number
    migratedMessages: number
    skipped: number
    errors: number
  }
}

async function main(): Promise<void> {
  const options = parseArgs(process.argv.slice(2))
  const dataDir = getDataDir()

  logger.info('='.repeat(60))
  logger.info('Starting legacy data migration')
  logger.info('='.repeat(60))
  logger.info({
    dataDir,
    dryRun: options.dryRun,
    deleteSource: options.deleteSource,
  })
  logger.info('')

  // Initialize database and run migrations
  const dbPath = path.join(dataDir, 'moatless.db')
  const database = createDrizzleDatabase(dbPath, logger)

  try {
    logger.info('Running database schema migrations...')
    await database.migrate()
    logger.info('✓ Database schema up to date\n')

    const stats: MigrationStats = {
      repositories: { total: 0, migrated: 0, skipped: 0, errors: 0 },
      tasks: { total: 0, migrated: 0, iterations: 0, skipped: 0, errors: 0 },
      messages: { total: 0, migratedTasks: 0, migratedMessages: 0, skipped: 0, errors: 0 },
    }

    // Migrate repositories
    await migrateRepositories(database, dataDir, options, stats)

    // Migrate tasks
    await migrateTasks(database, dataDir, options, stats)

    // Migrate messages
    await migrateMessages(database, dataDir, options, stats)

    // Print final summary
    printSummary(stats, options)
  } finally {
    await database.close()
  }
}

async function migrateRepositories(
  database: Database,
  dataDir: string,
  options: MigrationOptions,
  stats: MigrationStats,
): Promise<void> {
  logger.info('-'.repeat(60))
  logger.info('REPOSITORIES MIGRATION')
  logger.info('-'.repeat(60))

  const reposDir = path.join(dataDir, 'repositories')
  if (!(await directoryExists(reposDir))) {
    logger.info('No repositories/ directory found, skipping\n')
    return
  }

  const repository = new RepositoryRepository(database)
  const storage = new FileStorage(dataDir)

  try {
    const repoIds = await storage.list('repositories/')
    stats.repositories.total = repoIds.length

    if (repoIds.length === 0) {
      logger.info('No repositories found in storage\n')
      return
    }

    logger.info(`Found ${repoIds.length} repositories`)

    for (const key of repoIds) {
      const repoId = key.replace('repositories/', '')

      try {
        const repo = await storage.get<RepositoryConfig>(key)

        if (!repo) {
          logger.warn(`Repository ${repoId} not found in storage`)
          stats.repositories.skipped++
          continue
        }

        // Check if already exists
        const exists = await repository.exists(repoId)
        if (exists) {
          logger.debug(`Repository ${repoId} already in database`)
          stats.repositories.skipped++
          continue
        }

        if (options.dryRun) {
          logger.info(`[DRY RUN] Would migrate: ${repo.name}`)
          stats.repositories.migrated++
        } else {
          await repository.save(repo)
          logger.info(`✓ Migrated: ${repo.name}`)
          stats.repositories.migrated++

          if (options.deleteSource) {
            await storage.delete(key)
          }
        }
      } catch (error) {
        logger.error(`✗ Failed to migrate repository ${repoId}`, error as Error)
        stats.repositories.errors++
      }
    }

    logger.info('')
  } catch (error) {
    logger.error('Repository migration failed', error as Error)
    throw error
  }
}

async function migrateTasks(
  database: Database,
  dataDir: string,
  options: MigrationOptions,
  stats: MigrationStats,
): Promise<void> {
  logger.info('-'.repeat(60))
  logger.info('TASKS MIGRATION')
  logger.info('-'.repeat(60))

  const tasksDir = path.join(dataDir, 'tasks')
  if (!(await directoryExists(tasksDir))) {
    logger.info('No tasks/ directory found, skipping\n')
    return
  }

  const repository = new TaskRepository(database)
  const storage = new FileStorage(dataDir)

  try {
    const files = await collectJsonFiles(tasksDir)
    stats.tasks.total = files.length

    if (files.length === 0) {
      logger.info('No task JSON files found\n')
      return
    }

    logger.info(`Found ${files.length} task files`)

    for (const filePath of files) {
      const taskId = path.basename(filePath, '.json')

      try {
        const task = await storage.get<Task>(`tasks/${taskId}`)

        if (!task) {
          logger.debug(`Task ${taskId} not found`)
          stats.tasks.skipped++
          continue
        }

        // Check if already exists
        const exists = await repository.exists(taskId)
        if (exists) {
          logger.debug(`Task ${taskId} already in database`)
          stats.tasks.skipped++
          continue
        }

        // Normalize dates
        const normalized = normalizeTask(task)
        const iterationCount = normalized.iterations?.length ?? 0

        if (options.dryRun) {
          logger.info(`[DRY RUN] Would migrate: ${taskId} (${iterationCount} iterations)`)
          stats.tasks.migrated++
          stats.tasks.iterations += iterationCount
        } else {
          await repository.save(normalized)
          logger.info(`✓ Migrated: ${taskId} (${iterationCount} iterations)`)
          stats.tasks.migrated++
          stats.tasks.iterations += iterationCount

          if (options.deleteSource) {
            await fs.unlink(filePath)
          }
        }
      } catch (error) {
        logger.error(`✗ Failed to migrate task ${taskId}`, error as Error)
        stats.tasks.errors++
      }
    }

    logger.info('')
  } catch (error) {
    logger.error('Task migration failed', error as Error)
    throw error
  }
}

async function migrateMessages(
  database: Database,
  dataDir: string,
  options: MigrationOptions,
  stats: MigrationStats,
): Promise<void> {
  logger.info('-'.repeat(60))
  logger.info('MESSAGES MIGRATION')
  logger.info('-'.repeat(60))

  const messagesDir = path.join(dataDir, 'messages')
  if (!(await directoryExists(messagesDir))) {
    logger.info('No messages/ directory found, skipping\n')
    return
  }

  const repository = new TaskMessageRepository(database)
  const storage = new FileStorage(dataDir)

  try {
    const files = await collectJsonlFiles(messagesDir)
    stats.messages.total = files.length

    if (files.length === 0) {
      logger.info('No JSONL transcript files found\n')
      return
    }

    logger.info(`Found ${files.length} message transcript files`)

    for (const filePath of files) {
      const taskId = path.basename(filePath, '.jsonl')

      try {
        const messages = await storage.readJsonl<AgentMessage>(path.join('messages', taskId))

        if (messages.length === 0) {
          logger.debug(`Task ${taskId} has no messages`)
          stats.messages.skipped++
          continue
        }

        // Check if already migrated
        const existingCount = await repository.count(taskId)
        if (existingCount > 0) {
          logger.debug(`Task ${taskId} messages already in database (${existingCount} messages)`)
          stats.messages.skipped++
          continue
        }

        if (options.dryRun) {
          logger.info(`[DRY RUN] Would migrate: ${taskId} (${messages.length} messages)`)
          stats.messages.migratedTasks++
          stats.messages.migratedMessages += messages.length
        } else {
          await repository.appendMany(taskId, messages)
          logger.info(`✓ Migrated: ${taskId} (${messages.length} messages)`)
          stats.messages.migratedTasks++
          stats.messages.migratedMessages += messages.length

          if (options.deleteSource) {
            await fs.unlink(filePath)
          }
        }
      } catch (error) {
        logger.error(`✗ Failed to migrate messages for task ${taskId}`, error as Error)
        stats.messages.errors++
      }
    }

    logger.info('')
  } catch (error) {
    logger.error('Message migration failed', error as Error)
    throw error
  }
}

function printSummary(stats: MigrationStats, options: MigrationOptions): void {
  logger.info('='.repeat(60))
  logger.info('MIGRATION SUMMARY')
  logger.info('='.repeat(60))

  logger.info('\nRepositories:')
  logger.info(`  Total found:  ${stats.repositories.total}`)
  logger.info(`  Migrated:     ${stats.repositories.migrated}`)
  logger.info(`  Skipped:      ${stats.repositories.skipped}`)
  logger.info(`  Errors:       ${stats.repositories.errors}`)

  logger.info('\nTasks:')
  logger.info(`  Total found:  ${stats.tasks.total}`)
  logger.info(`  Migrated:     ${stats.tasks.migrated}`)
  logger.info(`  Iterations:   ${stats.tasks.iterations}`)
  logger.info(`  Skipped:      ${stats.tasks.skipped}`)
  logger.info(`  Errors:       ${stats.tasks.errors}`)

  logger.info('\nMessages:')
  logger.info(`  Total files:  ${stats.messages.total}`)
  logger.info(`  Tasks:        ${stats.messages.migratedTasks}`)
  logger.info(`  Messages:     ${stats.messages.migratedMessages}`)
  logger.info(`  Skipped:      ${stats.messages.skipped}`)
  logger.info(`  Errors:       ${stats.messages.errors}`)

  if (options.dryRun) {
    logger.info('\n*** DRY RUN MODE - No changes were made ***')
  } else if (options.deleteSource) {
    logger.info('\n*** Source files were deleted ***')
  }

  logger.info('='.repeat(60))
}

// Helper functions

function parseArgs(args: string[]): MigrationOptions {
  return {
    dryRun: args.includes('--dry-run'),
    deleteSource: args.includes('--delete-source'),
  }
}

async function directoryExists(dir: string): Promise<boolean> {
  try {
    const stat = await fs.stat(dir)
    return stat.isDirectory()
  } catch {
    return false
  }
}

async function collectJsonFiles(root: string): Promise<string[]> {
  const entries = await fs.readdir(root, { withFileTypes: true })
  const files: string[] = []

  for (const entry of entries) {
    const entryPath = path.join(root, entry.name)
    if (entry.isDirectory()) {
      const nested = await collectJsonFiles(entryPath)
      files.push(...nested)
    } else if (entry.isFile() && entry.name.endsWith('.json')) {
      files.push(entryPath)
    }
  }

  return files.sort()
}

async function collectJsonlFiles(root: string): Promise<string[]> {
  const entries = await fs.readdir(root, { withFileTypes: true })
  const files: string[] = []

  for (const entry of entries) {
    const entryPath = path.join(root, entry.name)
    if (entry.isDirectory()) {
      const nested = await collectJsonlFiles(entryPath)
      files.push(...nested)
    } else if (entry.isFile() && entry.name.endsWith('.jsonl')) {
      files.push(entryPath)
    }
  }

  return files.sort()
}

function normalizeTask(task: Task): Task {
  return {
    ...task,
    createdAt: task.createdAt instanceof Date ? task.createdAt : new Date(task.createdAt),
    lastMessageAt:
      task.lastMessageAt instanceof Date ? task.lastMessageAt : new Date(task.lastMessageAt),
    iterations: task.iterations?.map(normalizeIteration) ?? [],
  }
}

function normalizeIteration(iteration: Task['iterations'][number]): Task['iterations'][number] {
  return {
    ...iteration,
    startTime:
      iteration.startTime instanceof Date ? iteration.startTime : new Date(iteration.startTime),
    endTime: iteration.endTime
      ? iteration.endTime instanceof Date
        ? iteration.endTime
        : new Date(iteration.endTime)
      : undefined,
  }
}

// Run migration
main().catch((error) => {
  logger.error('Migration failed', error instanceof Error ? error : new Error(String(error)))
  process.exit(1)
})
