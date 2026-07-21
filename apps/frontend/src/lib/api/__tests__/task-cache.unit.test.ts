import type { UiMessage } from '@moatless/api-client'
import { QueryClient } from '@tanstack/react-query'
import { beforeEach, describe, expect, it } from 'vitest'
import { queryKeys } from '@/lib/api/query-keys'
import {
  applySubagentProgress,
  type MessagesInfiniteData,
  swapMessageUuid,
  upsertMessageByUuid,
} from '@/lib/api/task-cache'

const TASK = 'task-1'

function msg(uuid: string, overrides: Partial<UiMessage> = {}): UiMessage {
  return {
    uuid,
    role: 'assistant',
    messageType: 'assistant',
    timestamp: '2026-07-21T00:00:00.000Z',
    content: [{ type: 'text', text: uuid }],
    ...overrides,
  } as UiMessage
}

function seed(qc: QueryClient, messages: UiMessage[]): void {
  const data: MessagesInfiniteData = {
    pages: [{ messages, hasMore: false, nextBeforeId: undefined }],
    pageParams: [undefined],
  }
  qc.setQueryData(queryKeys.tasks.messages(TASK), data)
}

function messagesIn(qc: QueryClient): UiMessage[] {
  const data = qc.getQueryData<MessagesInfiniteData>(queryKeys.tasks.messages(TASK))
  return data?.pages.flatMap((p) => p.messages) ?? []
}

describe('task-cache', () => {
  let qc: QueryClient

  beforeEach(() => {
    qc = new QueryClient()
  })

  describe('upsertMessageByUuid', () => {
    it('is a no-op when the task has no messages cache entry', () => {
      upsertMessageByUuid(qc, TASK, msg('a'))
      expect(qc.getQueryData(queryKeys.tasks.messages(TASK))).toBeUndefined()
    })

    it('appends a new Message to the newest page', () => {
      seed(qc, [msg('a')])
      upsertMessageByUuid(qc, TASK, msg('b'))
      expect(messagesIn(qc).map((m) => m.uuid)).toEqual(['a', 'b'])
    })

    it('is idempotent — the same uuid twice yields one row (replaced in place)', () => {
      seed(qc, [msg('a')])
      upsertMessageByUuid(qc, TASK, msg('b', { content: [{ type: 'text', text: 'first' }] }))
      upsertMessageByUuid(qc, TASK, msg('b', { content: [{ type: 'text', text: 'second' }] }))
      const rows = messagesIn(qc)
      expect(rows.map((m) => m.uuid)).toEqual(['a', 'b'])
      expect(rows[1]?.content[0]).toMatchObject({ text: 'second' })
    })

    it('matches an existing row by provider messageId when uuid differs', () => {
      seed(qc, [msg('temp', { messageId: 'msg_123' })])
      upsertMessageByUuid(qc, TASK, msg('real', { messageId: 'msg_123' }))
      expect(messagesIn(qc).map((m) => m.uuid)).toEqual(['real'])
    })
  })

  describe('applySubagentProgress', () => {
    const agentMsg = (uuid: string, toolUseId: string): UiMessage =>
      msg(uuid, {
        content: [{ type: 'tool_use', toolName: 'Agent', toolUseId, text: null }],
      })

    it('writes the absolute summary through onto the matching Agent block', () => {
      seed(qc, [agentMsg('a', 'toolu_1')])
      applySubagentProgress(qc, TASK, {
        toolUseId: 'toolu_1',
        messageCount: 5,
        tokens: 1234,
        status: 'running',
      })
      const block = messagesIn(qc)[0]?.content[0]
      expect(block?.subagentSummary).toEqual({
        messageCount: 5,
        tokens: 1234,
        status: 'running',
      })
    })

    it('is a no-op when the task has no messages cache entry', () => {
      applySubagentProgress(qc, TASK, {
        toolUseId: 'toolu_1',
        messageCount: 1,
        tokens: 1,
        status: 'running',
      })
      expect(qc.getQueryData(queryKeys.tasks.messages(TASK))).toBeUndefined()
    })

    it('is a no-op when no Agent block matches the tool-use id', () => {
      seed(qc, [agentMsg('a', 'toolu_1')])
      applySubagentProgress(qc, TASK, {
        toolUseId: 'other',
        messageCount: 9,
        tokens: 9,
        status: 'completed',
      })
      expect(messagesIn(qc)[0]?.content[0]?.subagentSummary).toBeUndefined()
    })
  })

  describe('swapMessageUuid', () => {
    it('rewrites a provisional uuid to the server-assigned one, leaving one row', () => {
      seed(qc, [msg('temp-1')])
      swapMessageUuid(qc, TASK, 'temp-1', 'real-1')
      expect(messagesIn(qc).map((m) => m.uuid)).toEqual(['real-1'])
    })

    it('makes a later upsert of the real uuid idempotent (no duplicate)', () => {
      seed(qc, [msg('temp-1')])
      swapMessageUuid(qc, TASK, 'temp-1', 'real-1')
      upsertMessageByUuid(qc, TASK, msg('real-1'))
      expect(messagesIn(qc).map((m) => m.uuid)).toEqual(['real-1'])
    })
  })
})
