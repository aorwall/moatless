import type { AgentMessage, MessageContext } from '@moatless/types'
import { asc, eq, inArray } from 'drizzle-orm'
import type { BunSQLiteDatabase } from 'drizzle-orm/bun-sqlite'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import {
  agentMessageToInsert,
  mapContextRowToMessageContext,
  messageContextToInsert,
  selectToAgentMessage,
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

    const contextRows =
      messageIds.length > 0
        ? await this.db.drizzle
            .select()
            .from(taskMessageContexts)
            .where(inArray(taskMessageContexts.taskMessageId, messageIds))
            .orderBy(asc(taskMessageContexts.taskMessageId), asc(taskMessageContexts.id))
        : []

    const contextsByMessageId = new Map<number, MessageContext[]>()
    for (const row of contextRows) {
      const context = mapContextRowToMessageContext(row)
      if (context === null) {
        continue // Skip unknown context types
      }
      const list = contextsByMessageId.get(row.taskMessageId) ?? []
      list.push(context)
      contextsByMessageId.set(row.taskMessageId, list)
    }

    return rows.map((row) => {
      const message = selectToAgentMessage(row)
      const contexts = row.id ? contextsByMessageId.get(row.id) : undefined
      if (message.type === 'user' && contexts && contexts.length > 0) {
        message.contexts = contexts
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
      }
    })
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

  private async upsertMessage(
    tx: BunSQLiteDatabase,
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

  private async replaceContexts(
    tx: BunSQLiteDatabase,
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
}
