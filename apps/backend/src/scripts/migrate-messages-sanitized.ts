#!/usr/bin/env bun
/**
 * Migration script for messages with sanitization of usage fields
 *
 * This script reads JSONL message files and sanitizes the usage object
 * to only include fields that the database schema supports.
 */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { AgentMessage } from '@moatless/types'
import { TaskMessageRepository } from '../features/tasks/db/task-message.repository'
import { createDrizzleDatabase } from '../infrastructure/database/drizzle'
import { FileStorage } from '../storage/file-storage'
import { getDataDir } from '../utils/data-dir'
import { logger } from '../utils/logger'

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

function sanitizeMessage(message: AgentMessage): AgentMessage {
  // Create a clean copy of the message
  const sanitized = { ...message }

  // If it's an assistant message with usage, sanitize the usage object
  if (sanitized.type === 'assistant' && sanitized.message?.usage) {
    const usage = sanitized.message.usage as Record<string, unknown>
    sanitized.message = {
      ...sanitized.message,
      usage: {
        input_tokens: usage['input_tokens'] as number,
        output_tokens: usage['output_tokens'] as number,
        cache_creation_input_tokens: usage['cache_creation_input_tokens'] as number | undefined,
        cache_read_input_tokens: usage['cache_read_input_tokens'] as number | undefined,
        // Explicitly exclude: cache_creation, service_tier, and any other extra fields
      },
    }
  }

  return sanitized
}

async function main(): Promise<void> {
  const dataDir = getDataDir()
  const messagesDir = path.join(dataDir, 'messages')

  logger.info('Starting sanitized message migration')
  logger.info(`Data directory: ${dataDir}`)

  // Initialize database
  const dbPath = path.join(dataDir, 'moatless.db')
  const database = createDrizzleDatabase(dbPath, logger)

  try {
    const repository = new TaskMessageRepository(database)
    const storage = new FileStorage(dataDir)

    const files = await collectJsonlFiles(messagesDir)
    logger.info(`Found ${files.length} message transcript files`)

    let migratedTasks = 0
    let migratedMessages = 0
    let skipped = 0
    let errors = 0

    for (const filePath of files) {
      const taskId = path.basename(filePath, '.jsonl')

      try {
        const messages = await storage.readJsonl<AgentMessage>(path.join('messages', taskId))

        if (messages.length === 0) {
          logger.debug(`Task ${taskId} has no messages`)
          skipped++
          continue
        }

        // Check if already migrated
        const hasExisting = await repository.hasMessages(taskId)
        if (hasExisting) {
          logger.debug(`Task ${taskId} messages already in database`)
          skipped++
          continue
        }

        // Sanitize all messages
        const sanitizedMessages = messages.map(sanitizeMessage)

        // Insert messages one by one to identify which message fails
        for (let i = 0; i < sanitizedMessages.length; i++) {
          try {
            await repository.appendMany(taskId, [sanitizedMessages[i]])
          } catch (msgError) {
            logger.error(
              `Failed on message ${i + 1}/${sanitizedMessages.length} for task ${taskId}`,
              msgError as Error,
            )
            logger.error('Message data:', { message: sanitizedMessages[i] })
            throw msgError
          }
        }

        logger.info(`✓ Migrated: ${taskId} (${sanitizedMessages.length} messages)`)
        migratedTasks++
        migratedMessages += sanitizedMessages.length
      } catch (error) {
        logger.error(`✗ Failed to migrate messages for task ${taskId}`, error as Error)
        errors++
      }
    }

    logger.info('\n=== Migration Summary ===')
    logger.info(`Total files: ${files.length}`)
    logger.info(`Tasks migrated: ${migratedTasks}`)
    logger.info(`Messages migrated: ${migratedMessages}`)
    logger.info(`Skipped: ${skipped}`)
    logger.info(`Errors: ${errors}`)
  } finally {
    await database.close()
  }
}

main().catch((error) => {
  logger.error('Migration failed', error instanceof Error ? error : new Error(String(error)))
  process.exit(1)
})
