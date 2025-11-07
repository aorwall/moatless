import type {
  AgentMessage,
  AssistantAgentMessage,
  Attachment,
  MessageContext,
  ToolUseResult,
  UserAgentMessage,
} from '@moatless/types'
import { sql } from 'drizzle-orm'
import { index, integer, sqliteTable, text, uniqueIndex } from 'drizzle-orm/sqlite-core'
import { logger } from '../../../utils/logger'

type UserMessageContent = UserAgentMessage['message']['content']
type AssistantMessageContent = AssistantAgentMessage['message']['content']
type AssistantMessageUsage = AssistantAgentMessage['message']['usage']
type MessageContent = UserMessageContent | AssistantMessageContent
type ElementMessageContext = Extract<MessageContext, { type: 'element' }>
type FigmaMessageContext = Extract<MessageContext, { type: 'figma' }>
type PreviewUrlMessageContext = Extract<MessageContext, { type: 'previewUrl' }>

export const taskMessages = sqliteTable(
  'task_messages',
  {
    id: integer('id').primaryKey({ autoIncrement: true }),
    taskId: text('task_id').notNull(),
    uuid: text('uuid').notNull(),
    messageType: text('message_type').notNull(),
    userType: text('user_type').notNull(),
    parentUuid: text('parent_uuid'),
    isSidechain: integer('is_sidechain', { mode: 'boolean' }),
    cwd: text('cwd'),
    sessionId: text('session_id'),
    iterationNumber: integer('iteration_number'),
    version: text('version'),
    gitBranch: text('git_branch'),
    requestId: text('request_id'),
    timestamp: text('timestamp').notNull(),
    messageRole: text('message_role').notNull(),
    messageContent: text('message_content', { mode: 'json' }).$type<MessageContent>().notNull(),
    assistantMessageId: text('assistant_message_id'),
    assistantMessageType: text('assistant_message_type'),
    assistantMessageModel: text('assistant_message_model'),
    assistantStopReason: text('assistant_stop_reason'),
    assistantStopSequence: text('assistant_stop_sequence'),
    assistantUsageInputTokens: integer('assistant_usage_input_tokens'),
    assistantUsageOutputTokens: integer('assistant_usage_output_tokens'),
    assistantUsageCacheCreationInputTokens: integer('assistant_usage_cache_creation_input_tokens'),
    assistantUsageCacheReadInputTokens: integer('assistant_usage_cache_read_input_tokens'),
    toolUseResult: text('tool_use_result', { mode: 'json' }).$type<ToolUseResult | undefined>(),
    attachments: text('attachments', { mode: 'json' }).$type<Attachment[] | undefined>(),
    createdAt: integer('created_at', { mode: 'timestamp' })
      .notNull()
      .default(sql`CURRENT_TIMESTAMP`),
  },
  (table) => ({
    taskIdx: index('task_messages_task_idx').on(table.taskId, table.timestamp, table.id),
    uuidIdx: uniqueIndex('task_messages_uuid_idx').on(table.uuid),
  }),
)

export const taskMessageContexts = sqliteTable(
  'task_message_contexts',
  {
    id: integer('id').primaryKey({ autoIncrement: true }),
    taskMessageId: integer('task_message_id')
      .notNull()
      .references(() => taskMessages.id, { onDelete: 'cascade' }),
    type: text('type').notNull(),
    referenceId: text('reference_id').notNull(),
    data: text('data', { mode: 'json' }).$type<MessageContext['data']>().notNull(),
    createdAt: integer('created_at', { mode: 'timestamp' })
      .notNull()
      .default(sql`CURRENT_TIMESTAMP`),
  },
  (table) => ({
    messageIdx: index('task_message_contexts_message_idx').on(table.taskMessageId),
    referenceIdx: index('task_message_contexts_reference_idx').on(table.referenceId),
  }),
)

export type TaskMessageInsert = typeof taskMessages.$inferInsert
export type TaskMessageSelect = typeof taskMessages.$inferSelect
export type TaskMessageContextInsert = typeof taskMessageContexts.$inferInsert
export type TaskMessageContextSelect = typeof taskMessageContexts.$inferSelect

export function agentMessageToInsert(taskId: string, message: AgentMessage): TaskMessageInsert {
  const insert = buildBaseInsert(taskId, message)

  if (isUserAgentMessage(message)) {
    applyUserInsertFields(insert, message)
  } else {
    applyAssistantInsertFields(insert, message)
  }

  return insert
}

export function selectToAgentMessage(row: TaskMessageSelect): AgentMessage {
  return row.messageType === 'user'
    ? mapUserRowToAgentMessage(row)
    : mapAssistantRowToAgentMessage(row)
}

export function messageContextToInsert(
  messageId: number,
  context: MessageContext,
): TaskMessageContextInsert {
  return {
    taskMessageId: messageId,
    type: context.type,
    referenceId: context.referenceId,
    data: context.data,
  }
}

export function mapContextRowToMessageContext(
  row: TaskMessageContextSelect,
): MessageContext | null {
  if (row.type === 'element') {
    return {
      type: 'element',
      referenceId: row.referenceId,
      data: row.data as ElementMessageContext['data'],
    }
  }

  if (row.type === 'figma') {
    return {
      type: 'figma',
      referenceId: row.referenceId,
      data: row.data as FigmaMessageContext['data'],
    }
  }

  if (row.type === 'previewUrl') {
    return {
      type: 'previewUrl',
      referenceId: row.referenceId,
      data: row.data as PreviewUrlMessageContext['data'],
    }
  }

  // Unknown context type - log error and ignore
  logger.error(`Unknown message context type: ${row.type}`, {
    id: row.id,
    type: row.type,
    referenceId: row.referenceId,
  })
  return null
}

function mapUserRowToAgentMessage(row: TaskMessageSelect): UserAgentMessage {
  const userMessage: UserAgentMessage = {
    uuid: row.uuid,
    type: 'user',
    userType: row.userType,
    timestamp: row.timestamp,
    message: {
      role: row.messageRole as UserAgentMessage['message']['role'],
      content: row.messageContent as UserMessageContent,
    },
  }

  applyCommonRowFields(row, userMessage)
  setIfNotNull(row.toolUseResult, (value) => {
    userMessage.toolUseResult = value
  })
  setIfNotNull(row.attachments, (value) => {
    userMessage.attachments = value
  })

  return userMessage
}

function mapAssistantRowToAgentMessage(row: TaskMessageSelect): AssistantAgentMessage {
  const assistantMessage = createAssistantBaseMessage(row)

  applyCommonRowFields(row, assistantMessage)
  setIfNotNull(row.requestId, (value) => {
    assistantMessage.requestId = value
  })
  applyAssistantMessageMetadata(row, assistantMessage)
  applyAssistantUsageFromRow(row, assistantMessage)

  return assistantMessage
}

function buildBaseInsert(taskId: string, message: AgentMessage): TaskMessageInsert {
  return {
    taskId,
    uuid: message.uuid,
    messageType: message.type,
    userType: message.userType,
    parentUuid: message.parentUuid ?? undefined,
    isSidechain: message.isSidechain ?? undefined,
    cwd: message.cwd ?? undefined,
    sessionId: message.sessionId ?? undefined,
    iterationNumber: message.iterationNumber ?? undefined,
    version: message.version ?? undefined,
    gitBranch: message.gitBranch ?? undefined,
    requestId: isAssistantAgentMessage(message) ? (message.requestId ?? undefined) : undefined,
    timestamp: message.timestamp,
    messageRole: message.message.role,
    messageContent: message.message.content as MessageContent,
  }
}

function applyUserInsertFields(insert: TaskMessageInsert, message: UserAgentMessage): void {
  setIfDefined(message.toolUseResult, (value) => {
    insert.toolUseResult = value
  })
  setIfDefined(message.attachments, (value) => {
    insert.attachments = value
  })
}

function applyAssistantInsertFields(
  insert: TaskMessageInsert,
  message: AssistantAgentMessage,
): void {
  insert.assistantMessageId = message.message.id ?? undefined
  insert.assistantMessageType = message.message.type
  insert.assistantMessageModel = message.message.model ?? undefined
  insert.assistantStopReason = message.message.stop_reason ?? null
  insert.assistantStopSequence = message.message.stop_sequence ?? null

  const usage = message.message.usage
  if (!usage) {
    return
  }

  insert.assistantUsageInputTokens = usage.input_tokens ?? null
  insert.assistantUsageOutputTokens = usage.output_tokens ?? null
  insert.assistantUsageCacheCreationInputTokens = usage.cache_creation_input_tokens ?? null
  insert.assistantUsageCacheReadInputTokens = usage.cache_read_input_tokens ?? null
}

function createAssistantBaseMessage(row: TaskMessageSelect): AssistantAgentMessage {
  return {
    uuid: row.uuid,
    type: 'assistant',
    userType: row.userType,
    timestamp: row.timestamp,
    message: {
      type: (row.assistantMessageType ?? 'message') as AssistantAgentMessage['message']['type'],
      role: row.messageRole as AssistantAgentMessage['message']['role'],
      content: row.messageContent as AssistantMessageContent,
    },
  }
}

function applyCommonRowFields(row: TaskMessageSelect, target: AgentMessage): void {
  setIfNotNull(row.parentUuid, (value) => {
    target.parentUuid = value
  })
  setIfNotNull(row.isSidechain, (value) => {
    target.isSidechain = value
  })
  setIfNotNull(row.cwd, (value) => {
    target.cwd = value
  })
  setIfNotNull(row.sessionId, (value) => {
    target.sessionId = value
  })
  setIfNotNull(row.iterationNumber, (value) => {
    target.iterationNumber = value
  })
  setIfNotNull(row.version, (value) => {
    target.version = value
  })
  setIfNotNull(row.gitBranch, (value) => {
    target.gitBranch = value
  })
}

function applyAssistantMessageMetadata(
  row: TaskMessageSelect,
  assistantMessage: AssistantAgentMessage,
): void {
  setIfNotNull(row.assistantMessageId, (value) => {
    assistantMessage.message.id = value
  })
  setIfNotNull(row.assistantMessageModel, (value) => {
    assistantMessage.message.model = value
  })
  setNullable(row.assistantStopReason, (value) => {
    assistantMessage.message.stop_reason = value
  })
  setNullable(row.assistantStopSequence, (value) => {
    assistantMessage.message.stop_sequence = value
  })
}

function applyAssistantUsageFromRow(
  row: TaskMessageSelect,
  assistantMessage: AssistantAgentMessage,
): void {
  if (!hasUsageValues(row)) {
    return
  }

  const usage: NonNullable<AssistantMessageUsage> = {
    input_tokens: row.assistantUsageInputTokens!,
    output_tokens: row.assistantUsageOutputTokens!,
  }

  setIfNotNull(row.assistantUsageCacheCreationInputTokens, (value) => {
    usage.cache_creation_input_tokens = value
  })
  setIfNotNull(row.assistantUsageCacheReadInputTokens, (value) => {
    usage.cache_read_input_tokens = value
  })

  assistantMessage.message.usage = usage
}

function hasUsageValues(row: TaskMessageSelect): boolean {
  return (
    row.assistantUsageInputTokens !== null &&
    row.assistantUsageInputTokens !== undefined &&
    row.assistantUsageOutputTokens !== null &&
    row.assistantUsageOutputTokens !== undefined
  )
}

function isUserAgentMessage(message: AgentMessage): message is UserAgentMessage {
  return message.type === 'user'
}

function isAssistantAgentMessage(message: AgentMessage): message is AssistantAgentMessage {
  return message.type === 'assistant'
}

function setIfDefined<T>(value: T | undefined, assign: (value: T) => void): void {
  if (value !== undefined) {
    assign(value)
  }
}

function setIfNotNull<T>(value: T | null | undefined, assign: (value: T) => void): void {
  if (value !== null && value !== undefined) {
    assign(value)
  }
}

function setNullable<T>(value: T | null | undefined, assign: (value: T | null) => void): void {
  if (value !== undefined) {
    assign(value ?? null)
  }
}
