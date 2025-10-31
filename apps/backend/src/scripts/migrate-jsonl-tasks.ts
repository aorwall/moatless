#!/usr/bin/env bun
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { Task } from '@moatless/types'
import { TaskRepository } from '../features/tasks/db/task.repository'
import type { Database } from '../infrastructure/database/database'
import { createDrizzleDatabase } from '../infrastructure/database/drizzle'
import { FileStorage } from '../storage/file-storage'
import { getDataDir } from '../utils/data-dir'
import { logger } from '../utils/logger'

interface MigrationOptions {
  dryRun: boolean
  deleteSource: boolean
}

async function main(): Promise<void> {
  const options = parseArgs(process.argv.slice(2))

  const dataDir = getDataDir()
  const tasksDir = path.join(dataDir, 'tasks')

  const exists = await directoryExists(tasksDir)
  if (!exists) {
    logger.info('No legacy tasks directory found, nothing to migrate', { tasksDir })
    return
  }

  const database = initializeDatabase()
  const repository = new TaskRepository(database)
  const storage = new FileStorage(dataDir)

  try {
    const files = await collectJsonFiles(tasksDir)
    if (files.length === 0) {
      logger.info('No JSON task files found', { tasksDir })
      return
    }

    let migratedTasks = 0
    let migratedIterations = 0

    for (const filePath of files) {
      const taskId = path.basename(filePath, '.json')
      const task = await loadTask(storage, taskId)

      if (!task) {
        logger.debug('Skipping invalid task', { taskId })
        continue
      }

      logger.info('Migrating task', {
        taskId,
        iterationCount: task.iterations?.length ?? 0,
        dryRun: options.dryRun,
      })

      if (!options.dryRun) {
        try {
          await repository.save(task)
          migratedTasks += 1
          migratedIterations += task.iterations?.length ?? 0

          if (options.deleteSource) {
            await fs.unlink(filePath)
          }
        } catch (error) {
          logger.error('Failed to persist task', {
            taskId,
            iterationCount: task.iterations?.length ?? 0,
            error,
          })
          throw error
        }
      }
    }

    logger.info('Task migration complete', {
      migratedTasks,
      migratedIterations,
      dryRun: options.dryRun,
      deleteSource: options.deleteSource,
    })
  } finally {
    await database.close()
  }
}

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

function initializeDatabase(): Database {
  const dataDir = getDataDir()
  const dbPath = path.join(dataDir, 'moatless.db')
  const db = createDrizzleDatabase(dbPath, logger)

  // Skip migrations - assume schema already exists from drizzle-kit push or previous migrations
  // This is a one-time data migration script, not a schema migration

  return db
}

async function loadTask(storage: FileStorage, taskId: string): Promise<Task | null> {
  try {
    const task = await storage.get<Task>(`tasks/${taskId}`)
    if (!task) {
      return null
    }

    // Normalize task data
    return normalizeTask(task)
  } catch (error) {
    logger.warn('Failed to load task', { taskId, error })
    return null
  }
}

/**
 * Normalize task data to ensure all required fields are present
 * and dates are properly converted.
 */
function normalizeTask(task: Task): Task {
  return {
    ...task,
    createdAt: task.createdAt instanceof Date ? task.createdAt : new Date(task.createdAt),
    lastMessageAt:
      task.lastMessageAt instanceof Date ? task.lastMessageAt : new Date(task.lastMessageAt),
    iterations: task.iterations?.map(normalizeIteration) ?? [],
  }
}

/**
 * Normalize iteration data to ensure dates are properly converted.
 */
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

main().catch((error) => {
  logger.error(
    'Failed to migrate JSONL tasks',
    error instanceof Error ? error : new Error(String(error)),
  )
  process.exitCode = 1
})
