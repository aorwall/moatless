#!/usr/bin/env bun
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  AgentMessage,
  AssistantAgentMessage,
  MessageContext,
  UserAgentMessage,
} from '@moatless/types'
import { TaskMessageRepository } from '../features/tasks/db/task-message.repository'
import { agentMessageToInsert } from '../features/tasks/db/task-message.schema'
import type { Database } from '../infrastructure/database/database'
import { createDrizzleDatabase } from '../infrastructure/database/drizzle'
import { FileStorage } from '../storage/file-storage'
import { getDataDir } from '../utils/data-dir'
import { logger } from '../utils/logger'

type UserAgentMessageContent = Exclude<UserAgentMessage['message']['content'], string>
type UserAgentMessageContentItem = UserAgentMessageContent[number]
type AssistantAgentMessageContent = AssistantAgentMessage['message']['content'][number]

interface MigrationOptions {
  dryRun: boolean
  deleteSource: boolean
}

async function main(): Promise<void> {
  const options = parseArgs(process.argv.slice(2))

  const dataDir = getDataDir()
  const messagesDir = path.join(dataDir, 'messages')

  const exists = await directoryExists(messagesDir)
  if (!exists) {
    logger.info('No legacy messages directory found, nothing to migrate', { messagesDir })
    return
  }

  const database = initializeDatabase()
  const repository = new TaskMessageRepository(database)
  const storage = new FileStorage(dataDir)

  try {
    const files = await collectJsonlFiles(messagesDir)
    if (files.length === 0) {
      logger.info('No JSONL transcript files found', { messagesDir })
      return
    }

    let migratedTasks = 0
    let migratedMessages = 0

    for (const filePath of files) {
      const taskId = path.basename(filePath, '.jsonl')
      const messages = await loadMessages(storage, taskId)

      if (messages.length === 0) {
        logger.debug('Skipping empty transcript', { taskId })
        continue
      }

      logger.info('Migrating transcript', {
        taskId,
        messageCount: messages.length,
        dryRun: options.dryRun,
      })

      if (!options.dryRun) {
        try {
          const previewInsert = agentMessageToInsert(taskId, messages[0]!)
          logger.debug('Preview insert payload', {
            taskId,
            insertPreview: previewInsert,
          })
          await repository.appendMany(taskId, messages)
          migratedTasks += 1
          migratedMessages += messages.length

          if (options.deleteSource) {
            await fs.unlink(filePath)
          }
        } catch (error) {
          logger.error('Failed to persist transcript', {
            taskId,
            messageCount: messages.length,
            sampleMessage: messages[0],
            error,
          })
          throw error
        }
      }
    }

    logger.info('Message migration complete', {
      migratedTasks,
      migratedMessages,
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

function initializeDatabase(): Database {
  const dataDir = getDataDir()
  const dbPath = path.join(dataDir, 'moatless.db')
  const db = createDrizzleDatabase(dbPath, logger)

  // Skip migrations - assume schema already exists from drizzle-kit push or previous migrations
  // This is a one-time data migration script, not a schema migration

  return db
}

main().catch((error) => {
  logger.error(
    'Failed to migrate JSONL messages',
    error instanceof Error ? error : new Error(String(error)),
  )
  process.exitCode = 1
})
async function loadMessages(storage: FileStorage, taskId: string): Promise<AgentMessage[]> {
  const messages = await storage.readJsonl<AgentMessage>(path.join('messages', taskId))
  return messages.map(normalizeAgentMessage)
}

function normalizeAgentMessage(message: AgentMessage): AgentMessage {
  normalizeCommonFields(message)
  normalizeFlagFields(message)

  if (message.type === 'user') {
    normalizeUserFields(message)
  } else {
    normalizeAssistantFields(message)
  }

  return message
}

function normalizeCommonFields(message: AgentMessage): void {
  message.parentUuid = message.parentUuid ?? undefined
  message.sessionId = message.sessionId ?? undefined
  message.cwd = message.cwd ?? undefined
  message.version = message.version ?? undefined
  message.gitBranch = message.gitBranch ?? undefined
  message.userType = message.userType ?? 'user'
  message.timestamp = normalizeTimestamp(message.timestamp)
}

function normalizeFlagFields(message: AgentMessage): void {
  if ('isSidechain' in message) {
    const normalized = normalizeBoolean(message.isSidechain)
    message.isSidechain = normalized ?? undefined
  }

  if ('iterationNumber' in message) {
    const normalized = normalizeNumber(message.iterationNumber)
    message.iterationNumber = normalized ?? undefined
  }
}

function normalizeUserFields(message: AgentMessage & { type: 'user' }): void {
  const normalizedMessage = normalizeUserMessagePayload(message.message)
  message.message = normalizedMessage

  if ('attachments' in message) {
    message.attachments = message.attachments ?? undefined
  }
  if ('contexts' in message) {
    message.contexts = normalizeMessageContexts(message.contexts)
  }
  if ('toolUseResult' in message) {
    message.toolUseResult = message.toolUseResult ?? undefined
  }
}

function normalizeAssistantFields(message: AgentMessage & { type: 'assistant' }): void {
  const normalizedMessage = normalizeAssistantMessagePayload(message.message)
  message.message = normalizedMessage

  if ('requestId' in message) {
    message.requestId =
      typeof message.requestId === 'string' && message.requestId.length > 0
        ? message.requestId
        : undefined
  }
}

function normalizeUserMessagePayload(payload: unknown): UserAgentMessage['message'] {
  const base: UserAgentMessage['message'] = {
    role: 'user',
    content: '',
  }

  if (!payload || typeof payload !== 'object') {
    return base
  }

  const maybe = payload as Partial<UserAgentMessage['message']> & Record<string, unknown>

  base.content = normalizeUserContent(maybe.content)

  return base
}

function normalizeAssistantMessagePayload(payload: unknown): AssistantAgentMessage['message'] {
  const base: AssistantAgentMessage['message'] = {
    type: 'message',
    role: 'assistant',
    content: [],
  }

  if (!payload || typeof payload !== 'object') {
    return base
  }

  const maybe = payload as Partial<AssistantAgentMessage['message']> & Record<string, unknown>

  base.content = normalizeAssistantContent(maybe.content)

  if ('id' in maybe && typeof maybe['id'] === 'string') {
    base.id = maybe['id'] as string
  }
  if ('model' in maybe && typeof maybe['model'] === 'string') {
    base.model = maybe['model'] as string
  }
  if ('stop_reason' in maybe) {
    base.stop_reason = maybe['stop_reason'] ?? null
  }
  if ('stop_sequence' in maybe) {
    base.stop_sequence = maybe['stop_sequence'] ?? null
  }
  if ('usage' in maybe && maybe['usage'] && typeof maybe['usage'] === 'object') {
    base.usage = maybe['usage'] as AssistantAgentMessage['message']['usage']
  }

  return base
}

function normalizeMessageContexts(value: unknown): MessageContext[] | undefined {
  if (!Array.isArray(value)) {
    return undefined
  }

  const normalized: MessageContext[] = []

  for (const raw of value) {
    if (!raw || typeof raw !== 'object') {
      continue
    }

    const context = raw as Record<string, unknown>
    const type = context['type']

    if (type === 'element') {
      const element = normalizeElementContext(context)
      if (element) {
        normalized.push(element)
      }
    } else if (type === 'figma') {
      const figma = normalizeFigmaContext(context)
      if (figma) {
        normalized.push(figma)
      }
    }
  }

  return normalized.length > 0 ? normalized : undefined
}

function normalizeElementContext(raw: Record<string, unknown>): MessageContext | undefined {
  const data = raw['data']
  if (!data || typeof data !== 'object') {
    return undefined
  }

  const payload = data as Record<string, unknown>
  const componentId = payload['componentId']
  if (typeof componentId !== 'string' || componentId.length === 0) {
    return undefined
  }

  const referenceSource = raw['referenceId']
  const referenceId =
    typeof referenceSource === 'string' && referenceSource.length > 0
      ? referenceSource
      : componentId

  return {
    type: 'element',
    referenceId,
    data: {
      componentId,
      path: typeof payload['path'] === 'string' ? (payload['path'] as string) : '',
      line: toNumber(payload['line']),
      column: toNumber(payload['column']),
      componentName:
        typeof payload['componentName'] === 'string' ? (payload['componentName'] as string) : '',
    },
  }
}

function normalizeFigmaContext(raw: Record<string, unknown>): MessageContext | undefined {
  const data = raw['data']
  if (!data || typeof data !== 'object') {
    return undefined
  }

  const payload = data as Record<string, unknown>
  const importId = payload['importId']
  if (typeof importId !== 'string' || importId.length === 0) {
    return undefined
  }

  const referenceSource = raw['referenceId']
  const referenceId =
    typeof referenceSource === 'string' && referenceSource.length > 0 ? referenceSource : importId

  return {
    type: 'figma',
    referenceId,
    data: {
      importId,
      fileName: typeof payload['fileName'] === 'string' ? (payload['fileName'] as string) : '',
      nodeName: typeof payload['nodeName'] === 'string' ? (payload['nodeName'] as string) : '',
      hasImage: Boolean(payload['hasImage']),
    },
  }
}

function toNumber(value: unknown): number {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value
  }

  const parsed = Number(value)
  return Number.isNaN(parsed) ? 0 : parsed
}

function normalizeUserContent(content: unknown): UserAgentMessage['message']['content'] {
  if (Array.isArray(content)) {
    const normalized = content
      .map((item) => normalizeUserContentItem(item))
      .filter((item): item is UserAgentMessageContentItem => item !== null)
    return normalized as UserAgentMessageContent
  }

  if (typeof content === 'string') {
    return content
  }

  if (content && typeof content === 'object') {
    const normalized = normalizeUserContentItem(content)
    return normalized ? [normalized] : ''
  }

  return ''
}

function normalizeUserContentItem(item: unknown): UserAgentMessageContentItem | null {
  if (!item || typeof item !== 'object') {
    return null
  }

  const data = item as Record<string, unknown>
  const type = typeof data['type'] === 'string' ? data['type'] : 'text'

  if (type === 'tool_result') {
    return {
      type: 'tool_result',
      tool_use_id: typeof data['tool_use_id'] === 'string' ? data['tool_use_id'] : 'unknown',
      content: typeof data['content'] === 'string' ? data['content'] : '',
      is_error: typeof data['is_error'] === 'boolean' ? data['is_error'] : undefined,
    }
  }

  return {
    type: 'text',
    text: typeof data['text'] === 'string' ? data['text'] : stringifyUnknown(item),
  }
}

function normalizeAssistantContent(content: unknown): AssistantAgentMessage['message']['content'] {
  if (Array.isArray(content)) {
    const normalized = content
      .map((item) => normalizeAssistantContentItem(item))
      .filter((item): item is AssistantAgentMessageContent => item !== null)
    return normalized as AssistantAgentMessage['message']['content']
  }

  if (typeof content === 'string') {
    return [{ type: 'text', text: content }]
  }

  if (content && typeof content === 'object') {
    const normalized = normalizeAssistantContentItem(content)
    return normalized ? [normalized] : []
  }

  return []
}

function normalizeAssistantContentItem(item: unknown): AssistantAgentMessageContent | null {
  if (!item || typeof item !== 'object') {
    return null
  }

  const data = item as Record<string, unknown>
  const type = typeof data['type'] === 'string' ? data['type'] : 'text'

  if (type === 'tool_use') {
    return {
      type: 'tool_use',
      id: typeof data['id'] === 'string' ? data['id'] : 'unknown',
      name: typeof data['name'] === 'string' ? data['name'] : 'unknown',
      input: isPlainObject(data['input']) ? (data['input'] as Record<string, unknown>) : {},
    }
  }

  return {
    type: 'text',
    text: typeof data['text'] === 'string' ? data['text'] : stringifyUnknown(item),
  }
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return Boolean(
    value && typeof value === 'object' && Object.getPrototypeOf(value) === Object.prototype,
  )
}

function stringifyUnknown(value: unknown): string {
  if (typeof value === 'string') {
    return value
  }
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}

function normalizeTimestamp(value: string | Date): string {
  if (value instanceof Date) {
    return value.toISOString()
  }

  const parsed = new Date(value)
  return Number.isNaN(parsed.getTime()) ? new Date().toISOString() : parsed.toISOString()
}

function normalizeBoolean(value: unknown): boolean | null {
  if (typeof value === 'boolean') {
    return value
  }
  if (typeof value === 'string') {
    if (value.toLowerCase() === 'true') {
      return true
    }
    if (value.toLowerCase() === 'false') {
      return false
    }
  }
  return null
}

function normalizeNumber(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value
  }
  if (typeof value === 'string' && value.length > 0) {
    const parsed = Number(value)
    return Number.isFinite(parsed) ? parsed : null
  }
  return null
}
