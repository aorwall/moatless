import type {
  AgentMessage,
  MessageContentBlock,
  MessageContext,
  ResultAgentMessage,
  UserAgentMessage,
} from '@moatless/types'
import { and, asc, desc, eq, inArray, isNotNull, isNull, sql } from 'drizzle-orm'
import type { PostgresJsDatabase } from 'drizzle-orm/postgres-js'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import {
  agentMessageToInsert,
  contentBlockToInsert,
  mapContentBlockRowToMessageContentBlock,
  mapContextRowToMessageContext,
  messageContextToInsert,
  selectToAgentMessage,
  taskMessageContentBlocks,
  taskMessageContexts,
  taskMessages,
} from './task-message.schema'

@injectable()
export class TaskMessageRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  async getMessages(taskId: string): Promise<AgentMessage[]> {
    const rows = await this.db.drizzle
      .select()
      .from(taskMessages)
      .where(eq(taskMessages.taskId, taskId))
      .orderBy(asc(taskMessages.timestamp), asc(taskMessages.id))

    if (rows.length === 0) {
      return []
    }

    const messageIds = rows
      .map((row) => row.id)
      .filter((id): id is number => id !== null && id !== undefined)

    const [contextsByMessageId, contentBlocksByMessageId] = await Promise.all([
      this.loadContextsByMessageId(messageIds),
      this.loadContentBlocksByMessageId(messageIds),
    ])

    return rows.map((row) => {
      const message = selectToAgentMessage(row)
      if (message.type === 'user') {
        const contexts = row.id ? contextsByMessageId.get(row.id) : undefined
        if (contexts && contexts.length > 0) {
          message.contexts = contexts
        }
        const blocks = row.id ? contentBlocksByMessageId.get(row.id) : undefined
        if (blocks && blocks.length > 0) {
          message.contentBlocks = blocks
        }
      }
      return message
    })
  }

  async append(taskId: string, message: AgentMessage): Promise<void> {
    await this.db.transaction(async (tx) => {
      const messageId = await this.upsertMessage(tx, taskId, message)
      if (!messageId) {
        return
      }

      await this.replaceContexts(tx, messageId, this.extractContexts(message))
      await this.replaceContentBlocks(tx, messageId, this.extractContentBlocks(message))
    })
  }

  async appendMany(taskId: string, messages: AgentMessage[]): Promise<void> {
    if (messages.length === 0) {
      return
    }

    await this.db.transaction(async (tx) => {
      for (const message of messages) {
        const messageId = await this.upsertMessage(tx, taskId, message)
        if (!messageId) {
          continue
        }

        await this.replaceContexts(tx, messageId, this.extractContexts(message))
        await this.replaceContentBlocks(tx, messageId, this.extractContentBlocks(message))
      }
    })
  }

  /**
   * Delete a queued message (one that hasn't been sent to the agent yet).
   * The agentSessionId IS NULL guard prevents deleting already-sent messages.
   */
  async deleteQueuedMessage(taskId: string, messageUuid: string): Promise<boolean> {
    const deleted = await this.db.drizzle
      .delete(taskMessages)
      .where(
        and(
          eq(taskMessages.taskId, taskId),
          eq(taskMessages.uuid, messageUuid),
          isNull(taskMessages.agentSessionId),
        ),
      )
      .returning({ id: taskMessages.id })

    return deleted.length > 0
  }

  async deleteByTask(taskId: string): Promise<void> {
    await this.db.drizzle.delete(taskMessages).where(eq(taskMessages.taskId, taskId))
  }

  async hasMessages(taskId: string): Promise<boolean> {
    const rows = await this.db.drizzle
      .select({ id: taskMessages.id })
      .from(taskMessages)
      .where(eq(taskMessages.taskId, taskId))
      .limit(1)

    return rows.length > 0
  }

  /**
   * Get the last result message for a task.
   * Result messages carry turn completion status, metrics, and resume info.
   */
  async getLastResultMessage(taskId: string): Promise<ResultAgentMessage | null> {
    const rows = await this.db.drizzle
      .select()
      .from(taskMessages)
      .where(and(eq(taskMessages.taskId, taskId), eq(taskMessages.messageType, 'result')))
      .orderBy(desc(taskMessages.timestamp), desc(taskMessages.id))
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToAgentMessage(row) as ResultAgentMessage
  }

  /**
   * Get the last user message for a task.
   * Used to determine agentMode and model for the current/previous turn.
   */
  async getLastUserMessage(taskId: string): Promise<AgentMessage | null> {
    const rows = await this.db.drizzle
      .select()
      .from(taskMessages)
      .where(and(eq(taskMessages.taskId, taskId), eq(taskMessages.messageType, 'user')))
      .orderBy(desc(taskMessages.timestamp), desc(taskMessages.id))
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToAgentMessage(row)
  }

  /**
   * Get messages that haven't been acknowledged by the agent yet.
   * These are messages where agentSessionId is null.
   */
  async getMessagesWithoutAgentSessionId(taskId: string): Promise<AgentMessage[]> {
    const rows = await this.db.drizzle
      .select()
      .from(taskMessages)
      .where(and(eq(taskMessages.taskId, taskId), isNull(taskMessages.agentSessionId)))
      .orderBy(asc(taskMessages.timestamp), asc(taskMessages.id))

    if (rows.length === 0) {
      return []
    }

    const messageIds = rows
      .map((row) => row.id)
      .filter((id): id is number => id !== null && id !== undefined)

    const [contextsByMessageId, contentBlocksByMessageId] = await Promise.all([
      this.loadContextsByMessageId(messageIds),
      this.loadContentBlocksByMessageId(messageIds),
    ])

    return rows.map((row) => {
      const message = selectToAgentMessage(row)
      if (message.type === 'user') {
        const contexts = row.id ? contextsByMessageId.get(row.id) : undefined
        if (contexts && contexts.length > 0) {
          message.contexts = contexts
        }
        const blocks = row.id ? contentBlocksByMessageId.get(row.id) : undefined
        if (blocks && blocks.length > 0) {
          message.contentBlocks = blocks
        }
      }
      return message
    })
  }

  /**
   * Get pending user messages for a task (where agentSessionId IS NULL and type='user').
   * These are messages that haven't been sent to the agent yet.
   * Ordered by timestamp ASC for delivery in order.
   */
  async getPendingUserMessages(taskId: string): Promise<UserAgentMessage[]> {
    const rows = await this.db.drizzle
      .select()
      .from(taskMessages)
      .where(
        and(
          eq(taskMessages.taskId, taskId),
          isNull(taskMessages.agentSessionId),
          eq(taskMessages.messageType, 'user'),
        ),
      )
      .orderBy(asc(taskMessages.timestamp), asc(taskMessages.id))

    if (rows.length === 0) {
      return []
    }

    const messageIds = rows
      .map((row) => row.id)
      .filter((id): id is number => id !== null && id !== undefined)

    const [contextsByMessageId, contentBlocksByMessageId] = await Promise.all([
      this.loadContextsByMessageId(messageIds),
      this.loadContentBlocksByMessageId(messageIds),
    ])

    return rows.map((row) => {
      const message = selectToAgentMessage(row) as UserAgentMessage
      const contexts = row.id ? contextsByMessageId.get(row.id) : undefined
      if (contexts && contexts.length > 0) {
        message.contexts = contexts
      }
      const blocks = row.id ? contentBlocksByMessageId.get(row.id) : undefined
      if (blocks && blocks.length > 0) {
        message.contentBlocks = blocks
      }
      return message
    })
  }

  /**
   * Get the latest non-null agentSessionId for a task.
   * Used to determine the resume session ID when sending new messages.
   */
  async getLatestAgentSessionId(taskId: string): Promise<string | null> {
    const rows = await this.db.drizzle
      .select({ agentSessionId: taskMessages.agentSessionId })
      .from(taskMessages)
      .where(and(eq(taskMessages.taskId, taskId), isNotNull(taskMessages.agentSessionId)))
      .orderBy(desc(taskMessages.timestamp), desc(taskMessages.id))
      .limit(1)

    return rows[0]?.agentSessionId ?? null
  }

  /**
   * Update all messages without agentSessionId to have the given sessionId.
   * Also sets sentAt to now(), marking the messages as delivered to the agent.
   * Called when we receive the first user/system event from the agent,
   * indicating it has acknowledged our messages.
   */
  async updateUnacknowledgedMessages(taskId: string, agentSessionId: string): Promise<number> {
    const updated = await this.db.drizzle
      .update(taskMessages)
      .set({ agentSessionId, sentAt: sql`now()` })
      .where(and(eq(taskMessages.taskId, taskId), isNull(taskMessages.agentSessionId)))
      .returning({ id: taskMessages.id })

    return updated.length
  }

  private async upsertMessage(
    tx: PostgresJsDatabase,
    taskId: string,
    message: AgentMessage,
  ): Promise<number | null> {
    const existingIds = await tx
      .select({ id: taskMessages.id })
      .from(taskMessages)
      .where(eq(taskMessages.uuid, message.uuid))
      .limit(1)

    if (existingIds[0]?.id) {
      return existingIds[0].id
    }

    const insert = agentMessageToInsert(taskId, message)

    await tx.insert(taskMessages).values(insert).onConflictDoNothing({ target: taskMessages.uuid })

    const insertedIds = await tx
      .select({ id: taskMessages.id })
      .from(taskMessages)
      .where(eq(taskMessages.uuid, message.uuid))
      .limit(1)

    return insertedIds[0]?.id ?? null
  }

  private extractContexts(message: AgentMessage): MessageContext[] | undefined {
    return message.type === 'user' ? message.contexts : undefined
  }

  private extractContentBlocks(message: AgentMessage): MessageContentBlock[] | undefined {
    return message.type === 'user' ? message.contentBlocks : undefined
  }

  private async replaceContexts(
    tx: PostgresJsDatabase,
    messageId: number,
    contexts?: MessageContext[],
  ): Promise<void> {
    await tx.delete(taskMessageContexts).where(eq(taskMessageContexts.taskMessageId, messageId))

    if (!contexts || contexts.length === 0) {
      return
    }

    const inserts = contexts.map((context) => messageContextToInsert(messageId, context))

    await tx.insert(taskMessageContexts).values(inserts)
  }

  private async replaceContentBlocks(
    tx: PostgresJsDatabase,
    messageId: number,
    blocks?: MessageContentBlock[],
  ): Promise<void> {
    await tx
      .delete(taskMessageContentBlocks)
      .where(eq(taskMessageContentBlocks.taskMessageId, messageId))

    if (!blocks || blocks.length === 0) {
      return
    }

    const inserts = blocks.map((block) => contentBlockToInsert(messageId, block))

    await tx.insert(taskMessageContentBlocks).values(inserts)
  }

  private async loadContextsByMessageId(
    messageIds: number[],
  ): Promise<Map<number, MessageContext[]>> {
    const map = new Map<number, MessageContext[]>()
    if (messageIds.length === 0) {
      return map
    }

    const contextRows = await this.db.drizzle
      .select()
      .from(taskMessageContexts)
      .where(inArray(taskMessageContexts.taskMessageId, messageIds))
      .orderBy(asc(taskMessageContexts.taskMessageId), asc(taskMessageContexts.id))

    for (const row of contextRows) {
      const context = mapContextRowToMessageContext(row)
      if (context === null) {
        continue
      }
      const list = map.get(row.taskMessageId) ?? []
      list.push(context)
      map.set(row.taskMessageId, list)
    }

    return map
  }

  private async loadContentBlocksByMessageId(
    messageIds: number[],
  ): Promise<Map<number, MessageContentBlock[]>> {
    const map = new Map<number, MessageContentBlock[]>()
    if (messageIds.length === 0) {
      return map
    }

    const blockRows = await this.db.drizzle
      .select()
      .from(taskMessageContentBlocks)
      .where(inArray(taskMessageContentBlocks.taskMessageId, messageIds))
      .orderBy(asc(taskMessageContentBlocks.taskMessageId), asc(taskMessageContentBlocks.sortOrder))

    for (const row of blockRows) {
      const block = mapContentBlockRowToMessageContentBlock(row)
      const list = map.get(row.taskMessageId) ?? []
      list.push(block)
      map.set(row.taskMessageId, list)
    }

    return map
  }
}
