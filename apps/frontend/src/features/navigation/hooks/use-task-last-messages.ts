import { useQueries } from '@tanstack/react-query'
import { useMemo } from 'react'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'
import { type MessagePreview, previewFromUiMessages } from './message-preview'

/** How many recent messages to pull per task when sourcing a preview. A few is
 * enough to skip trailing tool/result rows and still find the last prose. */
const PREVIEW_FETCH_LIMIT = 6

/** Keep previews cached long enough to avoid refetching on every sidebar mount;
 * live updates come from the event stream, not from re-polling this endpoint. */
const PREVIEW_STALE_TIME_MS = 5 * 60 * 1000

/**
 * Fetch the latest-message preview for each task via the existing per-task
 * messages endpoint (`GET /tasks/:id/messages`). One request per task, cached
 * and shared through TanStack Query. Returns a `taskId -> preview` map that the
 * sidebar merges under live stream activity for idle tasks.
 *
 * Only pass the ids that actually need a fetched preview (e.g. those without a
 * live streamed message) to keep the request fan-out minimal.
 */
export function useTaskLastMessages(taskIds: string[]): Map<string, MessagePreview> {
  const results = useQueries({
    queries: taskIds.map((taskId) => ({
      queryKey: queryKeys.tasks.lastMessage(taskId),
      queryFn: () => tasksApi.getTaskMessages(taskId, { limit: PREVIEW_FETCH_LIMIT }),
      staleTime: PREVIEW_STALE_TIME_MS,
      select: (page: Awaited<ReturnType<typeof tasksApi.getTaskMessages>>) =>
        previewFromUiMessages(page.messages),
    })),
  })

  return useMemo(() => {
    const map = new Map<string, MessagePreview>()
    taskIds.forEach((taskId, index) => {
      const preview = results[index]?.data
      if (preview) {
        map.set(taskId, preview)
      }
    })
    return map
  }, [taskIds, results])
}
