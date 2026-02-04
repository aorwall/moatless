import type {
  AgentMessage,
  AssistantAgentMessage,
  Attachment,
  MessageContentBlock,
  MessageContext,
  ResultAgentMessage,
  RunMetrics,
  SystemAgentMessage,
  ToolUseResult,
  UserAgentMessage,
} from '@moatless/types'
import { sql } from 'drizzle-orm'
import {
  boolean,
  index,
  integer,
  jsonb,
  pgTable,
  serial,
  text,
  timestamp,
  uniqueIndex,
} from 'drizzle-orm/pg-core'
import { logger } from '../../../utils/logger'

type UserMessageContent = UserAgentMessage['message']['content']
type SystemMessageContent = SystemAgentMessage['message']['content']
type AssistantMessageContent = AssistantAgentMessage['message']['content']
type AssistantMessageUsage = AssistantAgentMessage['message']['usage']
type MessageContent = UserMessageContent | SystemMessageContent | AssistantMessageContent
type ElementMessageContext = Extract<MessageContext, { type: 'element' }>
type FigmaMessageContext = Extract<MessageContext, { type: 'figma' }>
type PreviewUrlMessageContext = Extract<MessageContext, { type: 'previewUrl' }>

export const taskMessages = pgTable(
  'task_messages',
  {
    id: serial('id').primaryKey(),
    taskId: text('task_id').notNull(),
    uuid: text('uuid').notNull(),
    messageType: text('message_type').notNull(),
    userType: text('user_type').notNull(),
    parentUuid: text('parent_uuid'),
    isSidechain: boolean('is_sidechain'),
    cwd: text('cwd'),
    sessionId: text('session_id'),
    turnNumber: integer('turn_number'),
    version: text('version'),
    gitBranch: text('git_branch'),
    requestId: text('request_id'),
    timestamp: text('timestamp').notNull(),
    messageRole: text('message_role').notNull(),
    messageContent: jsonb('message_content').$type<MessageContent>().notNull(),
    assistantMessageId: text('assistant_message_id'),
    assistantMessageType: text('assistant_message_type'),
    assistantMessageModel: text('assistant_message_model'),
    assistantStopReason: text('assistant_stop_reason'),
    assistantStopSequence: text('assistant_stop_sequence'),
    assistantUsageInputTokens: integer('assistant_usage_input_tokens'),
    assistantUsageOutputTokens: integer('assistant_usage_output_tokens'),
    assistantUsageCacheCreationInputTokens: integer('assistant_usage_cache_creation_input_tokens'),
    assistantUsageCacheReadInputTokens: integer('assistant_usage_cache_read_input_tokens'),
    toolUseResult: jsonb('tool_use_result').$type<ToolUseResult | undefined>(),
    attachments: jsonb('attachments').$type<Attachment[] | undefined>(),
    // Agent session ID - set when agent acknowledges receipt of this message
    // Null means message hasn't been received by agent yet (pending acknowledgment)
    agentSessionId: text('agent_session_id'),
    // Result message fields (populated for type='result' messages)
    turnStatus: text('turn_status'), // 'completed' | 'failed'
    turnResult: text('turn_result'),
    willNotResume: boolean('will_not_resume'),
    runMetrics: jsonb('run_metrics').$type<RunMetrics | undefined>(),
    // User message fields (populated for type='user' messages)
    agentMode: text('agent_mode'), // 'edit' | 'plan'
    model: text('model'), // Model used for this turn
    resumeSessionId: text('resume_session_id'), // Session ID to resume from
    // Timestamp when the message was actually sent/delivered to the agent
    // Null means message hasn't been sent to agent yet
    sentAt: timestamp('sent_at'),
    createdAt: timestamp('created_at').notNull().default(sql`now()`),
  },
  (table) => ({
    taskIdx: index('task_messages_task_idx').on(table.taskId, table.timestamp, table.id),
    uuidIdx: uniqueIndex('task_messages_uuid_idx').on(table.uuid),
    // Index for efficiently finding unacknowledged messages
    unacknowledgedIdx: index('task_messages_unacknowledged_idx').on(
      table.taskId,
      table.agentSessionId,
    ),
  }),
)

export const taskMessageContexts = pgTable(
  'task_message_contexts',
  {
    id: serial('id').primaryKey(),
    taskMessageId: integer('task_message_id')
      .notNull()
      .references(() => taskMessages.id, { onDelete: 'cascade' }),
    type: text('type').notNull(),
    referenceId: text('reference_id').notNull(),
    data: jsonb('data').$type<MessageContext['data']>().notNull(),
    createdAt: timestamp('created_at').notNull().default(sql`now()`),
  },
  (table) => ({
    messageIdx: index('task_message_contexts_message_idx').on(table.taskMessageId),
    referenceIdx: index('task_message_contexts_reference_idx').on(table.referenceId),
  }),
)

export const taskMessageContentBlocks = pgTable(
  'task_message_content_blocks',
  {
    id: serial('id').primaryKey(),
    taskMessageId: integer('task_message_id')
      .notNull()
      .references(() => taskMessages.id, { onDelete: 'cascade' }),
    sortOrder: integer('sort_order').notNull().default(0),
    blockType: text('block_type').notNull(), // 'text' | 'tool_result' | 'image' | 'system'
    showInUI: boolean('show_in_ui').notNull().default(true),
    // text block fields
    textContent: text('text_content'),
    // tool_result block fields
    toolUseId: text('tool_use_id'),
    toolResultContent: text('tool_result_content'),
    toolResultIsError: boolean('tool_result_is_error'),
    // image block fields
    imageMediaType: text('image_media_type'),
    imageData: text('image_data'),
    createdAt: timestamp('created_at').notNull().default(sql`now()`),
  },
  (table) => ({
    messageIdx: index('task_message_content_blocks_message_idx').on(table.taskMessageId),
  }),
)

export type TaskMessageInsert = typeof taskMessages.$inferInsert
export type TaskMessageSelect = typeof taskMessages.$inferSelect
export type TaskMessageContextInsert = typeof taskMessageContexts.$inferInsert
export type TaskMessageContextSelect = typeof taskMessageContexts.$inferSelect
export type TaskMessageContentBlockInsert = typeof taskMessageContentBlocks.$inferInsert
export type TaskMessageContentBlockSelect = typeof taskMessageContentBlocks.$inferSelect

export function agentMessageToInsert(taskId: string, message: AgentMessage): TaskMessageInsert {
  const insert = buildBaseInsert(taskId, message)

  if (isUserAgentMessage(message)) {
    applyUserInsertFields(insert, message)
  } else if (isSystemAgentMessage(message)) {
    applySystemInsertFields(insert, message)
  } else if (isResultAgentMessage(message)) {
    applyResultInsertFields(insert, message)
  } else {
    applyAssistantInsertFields(insert, message)
  }

  return insert
}

export function selectToAgentMessage(row: TaskMessageSelect): AgentMessage {
  if (row.messageType === 'user') {
    return mapUserRowToAgentMessage(row)
  }
  if (row.messageType === 'system') {
    return mapSystemRowToAgentMessage(row)
  }
  if (row.messageType === 'result') {
    return mapResultRowToAgentMessage(row)
  }
  return mapAssistantRowToAgentMessage(row)
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
  logger.error(
    {
      id: row.id,
      type: row.type,
      referenceId: row.referenceId,
    },
    `Unknown message context type: ${row.type}`,
  )
  return null
}

export function contentBlockToInsert(
  messageId: number,
  block: MessageContentBlock,
): TaskMessageContentBlockInsert {
  return {
    taskMessageId: messageId,
    sortOrder: block.sortOrder ?? 0,
    blockType: block.blockType,
    showInUI: block.showInUI ?? true,
    textContent: block.textContent ?? undefined,
    toolUseId: block.toolUseId ?? undefined,
    toolResultContent: block.toolResultContent ?? undefined,
    toolResultIsError: block.toolResultIsError ?? undefined,
    imageMediaType: block.imageMediaType ?? undefined,
    imageData: block.imageData ?? undefined,
  }
}

export function mapContentBlockRowToMessageContentBlock(
  row: TaskMessageContentBlockSelect,
): MessageContentBlock {
  return {
    sortOrder: row.sortOrder,
    blockType: row.blockType as MessageContentBlock['blockType'],
    showInUI: row.showInUI,
    textContent: row.textContent ?? undefined,
    toolUseId: row.toolUseId ?? undefined,
    toolResultContent: row.toolResultContent ?? undefined,
    toolResultIsError: row.toolResultIsError ?? undefined,
    imageMediaType: row.imageMediaType ?? undefined,
    imageData: row.imageData ?? undefined,
  }
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
  setIfNotNull(row.agentMode, (value) => {
    userMessage.agentMode = value as UserAgentMessage['agentMode']
  })
  setIfNotNull(row.model, (value) => {
    userMessage.model = value
  })
  setIfNotNull(row.resumeSessionId, (value) => {
    userMessage.resumeSessionId = value
  })

  return userMessage
}

function mapSystemRowToAgentMessage(row: TaskMessageSelect): SystemAgentMessage {
  const systemMessage: SystemAgentMessage = {
    uuid: row.uuid,
    type: 'system',
    userType: row.userType,
    timestamp: row.timestamp,
    message: {
      role: row.messageRole as SystemAgentMessage['message']['role'],
      content: row.messageContent as SystemMessageContent,
    },
  }

  applyCommonRowFields(row, systemMessage)
  setIfNotNull(row.toolUseResult, (value) => {
    systemMessage.toolUseResult = value
  })

  return systemMessage
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
  // Result messages don't have a message.role/content structure
  const hasMessage = 'message' in message && message.message != null
  return {
    taskId,
    uuid: message.uuid,
    messageType: message.type,
    userType: message.userType,
    parentUuid: message.parentUuid ?? undefined,
    isSidechain: message.isSidechain ?? undefined,
    cwd: message.cwd ?? undefined,
    sessionId: message.sessionId ?? undefined,
    turnNumber: message.turnNumber ?? undefined,
    version: message.version ?? undefined,
    gitBranch: message.gitBranch ?? undefined,
    requestId: isAssistantAgentMessage(message) ? (message.requestId ?? undefined) : undefined,
    timestamp: message.timestamp,
    messageRole: hasMessage
      ? (message as UserAgentMessage | SystemAgentMessage | AssistantAgentMessage).message.role
      : 'system',
    messageContent: hasMessage
      ? ((message as UserAgentMessage | SystemAgentMessage | AssistantAgentMessage).message
          .content as MessageContent)
      : ('{}' as unknown as MessageContent),
    agentSessionId: message.agentSessionId ?? undefined,
    sentAt: message.sentAt ? new Date(message.sentAt) : undefined,
  }
}

function applyUserInsertFields(insert: TaskMessageInsert, message: UserAgentMessage): void {
  setIfDefined(message.toolUseResult, (value) => {
    insert.toolUseResult = value
  })
  setIfDefined(message.attachments, (value) => {
    insert.attachments = value
  })
  setIfDefined(message.agentMode, (value) => {
    insert.agentMode = value
  })
  setIfDefined(message.model, (value) => {
    insert.model = value
  })
  setIfDefined(message.resumeSessionId, (value) => {
    insert.resumeSessionId = value
  })
}

function applySystemInsertFields(insert: TaskMessageInsert, message: SystemAgentMessage): void {
  setIfDefined(message.toolUseResult, (value) => {
    insert.toolUseResult = value
  })
}

function applyResultInsertFields(insert: TaskMessageInsert, message: ResultAgentMessage): void {
  insert.turnStatus = message.turnStatus
  insert.turnResult = message.turnResult ?? undefined
  insert.willNotResume = message.willNotResume ?? undefined
  insert.runMetrics = message.runMetrics ?? undefined
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
  setIfNotNull(row.turnNumber, (value) => {
    target.turnNumber = value
  })
  setIfNotNull(row.version, (value) => {
    target.version = value
  })
  setIfNotNull(row.gitBranch, (value) => {
    target.gitBranch = value
  })
  setIfNotNull(row.agentSessionId, (value) => {
    target.agentSessionId = value
  })
  setIfNotNull(row.sentAt, (value) => {
    target.sentAt = value instanceof Date ? value.toISOString() : String(value)
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

function mapResultRowToAgentMessage(row: TaskMessageSelect): ResultAgentMessage {
  const resultMessage: ResultAgentMessage = {
    uuid: row.uuid,
    type: 'result',
    userType: row.userType,
    timestamp: row.timestamp,
    turnStatus: row.turnStatus as ResultAgentMessage['turnStatus'],
  }

  applyCommonRowFields(row, resultMessage as unknown as AgentMessage)
  setIfNotNull(row.turnResult, (value) => {
    resultMessage.turnResult = value
  })
  setIfNotNull(row.willNotResume, (value) => {
    resultMessage.willNotResume = value
  })
  setIfNotNull(row.runMetrics, (value) => {
    resultMessage.runMetrics = value
  })

  return resultMessage
}

function isUserAgentMessage(message: AgentMessage): message is UserAgentMessage {
  return message.type === 'user'
}

function isSystemAgentMessage(message: AgentMessage): message is SystemAgentMessage {
  return message.type === 'system'
}

function isResultAgentMessage(message: AgentMessage): message is ResultAgentMessage {
  return message.type === 'result'
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
