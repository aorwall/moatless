/**
 * Shared cache-shape primitives for the task/message caches.
 *
 * One owner of "how a Message lives in the messages cache", called by both the
 * event path (message.upsert / message.deleted) and the optimistic mutations.
 * Keeps the two writer families from drifting on the infinite-query page model.
 */
import type { SubagentSummary, UiMessage } from '@moatless/api-client'
import type { InfiniteData, QueryClient } from '@tanstack/react-query'
import type { MessagesPage } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

export type MessagesInfiniteData = InfiniteData<MessagesPage, number | undefined>

/** Absolute subagent summary carried by a `message`/`subagentProgress` event. */
export interface SubagentProgress extends SubagentSummary {
  toolUseId: string
}

/** A Message matches an incoming one by persisted uuid, or by provider messageId. */
function sameMessage(existing: UiMessage, incoming: UiMessage): boolean {
  return (
    existing.uuid === incoming.uuid ||
    (incoming.messageId != null && existing.messageId === incoming.messageId)
  )
}

/**
 * Replace-or-insert a Message by uuid in an existing messages cache. No-op when
 * the task has no messages cache entry — the read path hydrates it on open, so
 * we never fabricate a partial history from a single event.
 */
export function upsertMessageByUuid(
  queryClient: QueryClient,
  taskId: string,
  message: UiMessage,
): void {
  const key = queryKeys.tasks.messages(taskId)
  const data = queryClient.getQueryData<MessagesInfiniteData>(key)
  if (!data || data.pages.length === 0) {
    return
  }

  let replaced = false
  const pages = data.pages.map((page) => {
    const idx = page.messages.findIndex((m) => sameMessage(m, message))
    if (idx === -1) {
      return page
    }
    replaced = true
    const messages = [...page.messages]
    messages[idx] = message
    return { ...page, messages }
  })

  if (replaced) {
    queryClient.setQueryData<MessagesInfiniteData>(key, { ...data, pages })
    return
  }

  // Not present yet — append to the newest (last) page.
  const lastPage = data.pages.at(-1)
  if (!lastPage) {
    return
  }
  const nextPages = [...data.pages]
  nextPages[nextPages.length - 1] = {
    ...lastPage,
    messages: [...lastPage.messages, message],
  }
  queryClient.setQueryData<MessagesInfiniteData>(key, { ...data, pages: nextPages })
}

/** Remove a Message by uuid from an existing messages cache (no-op if absent). */
export function removeMessage(queryClient: QueryClient, taskId: string, uuid: string): void {
  const key = queryKeys.tasks.messages(taskId)
  const data = queryClient.getQueryData<MessagesInfiniteData>(key)
  if (!data) {
    return
  }
  const pages = data.pages.map((page) => ({
    ...page,
    messages: page.messages.filter((m) => m.uuid !== uuid),
  }))
  queryClient.setQueryData<MessagesInfiniteData>(key, { ...data, pages })
}

/**
 * Write through an absolute subagent summary onto the cached parent Message's
 * `Agent` content block (the block whose `toolUseId` spawned the subagent).
 * Idempotent — a replayed or duplicated tick is a no-op; self-heals after a
 * reconnect via the read-path summary. No-op when the task has no messages
 * cache or no matching block is loaded.
 */
export function applySubagentProgress(
  queryClient: QueryClient,
  taskId: string,
  progress: SubagentProgress,
): void {
  const key = queryKeys.tasks.messages(taskId)
  const data = queryClient.getQueryData<MessagesInfiniteData>(key)
  if (!data || data.pages.length === 0) {
    return
  }

  const summary: SubagentSummary = {
    messageCount: progress.messageCount,
    tokens: progress.tokens,
    status: progress.status,
  }

  let changed = false
  const pages = data.pages.map((page) => {
    let pageChanged = false
    const messages = page.messages.map((message) => {
      const idx = message.content.findIndex(
        (block) => block.toolName === 'Agent' && block.toolUseId === progress.toolUseId,
      )
      const block = idx === -1 ? undefined : message.content[idx]
      if (!block) {
        return message
      }
      pageChanged = true
      const content = [...message.content]
      content[idx] = { ...block, subagentSummary: summary }
      return { ...message, content }
    })
    if (!pageChanged) {
      return page
    }
    changed = true
    return { ...page, messages }
  })

  if (changed) {
    queryClient.setQueryData<MessagesInfiniteData>(key, { ...data, pages })
  }
}

/** Swap a provisional (optimistic) Message's uuid for the server-assigned one. */
export function swapMessageUuid(
  queryClient: QueryClient,
  taskId: string,
  fromUuid: string,
  toUuid: string,
): void {
  const key = queryKeys.tasks.messages(taskId)
  const data = queryClient.getQueryData<MessagesInfiniteData>(key)
  if (!data) {
    return
  }
  const pages = data.pages.map((page) => ({
    ...page,
    messages: page.messages.map((m) => (m.uuid === fromUuid ? { ...m, uuid: toUuid } : m)),
  }))
  queryClient.setQueryData<MessagesInfiniteData>(key, { ...data, pages })
}
