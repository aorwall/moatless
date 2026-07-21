import type { UiMessage } from '@moatless/api-client'
import { QueryClient } from '@tanstack/react-query'
import { beforeEach, describe, expect, it } from 'vitest'
import { queryKeys } from '@/lib/api/query-keys'
import {
  type MessagesInfiniteData,
  removeMessage,
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

  describe('removeMessage', () => {
    it('removes a Message by uuid', () => {
      seed(qc, [msg('a'), msg('b')])
      removeMessage(qc, TASK, 'a')
      expect(messagesIn(qc).map((m) => m.uuid)).toEqual(['b'])
    })

    it('is a no-op when the uuid is absent', () => {
      seed(qc, [msg('a')])
      removeMessage(qc, TASK, 'zzz')
      expect(messagesIn(qc).map((m) => m.uuid)).toEqual(['a'])
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
