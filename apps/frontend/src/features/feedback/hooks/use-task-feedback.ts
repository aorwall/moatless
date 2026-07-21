import { useQuery } from '@tanstack/react-query'
import { type FeedbackResponse, feedbackApi } from '@/features/feedback/api/feedback.api'
import { queryKeys } from '@/lib/api/query-keys'

export function useTaskFeedback(taskId: string | undefined) {
  return useQuery({
    queryKey: queryKeys.feedback.byTask(taskId ?? ''),
    queryFn: () => feedbackApi.listForTask(taskId as string),
    enabled: !!taskId,
  })
}

export interface TaskFeedbackIndex {
  total: number
  byMessageUuid: Map<string, number>
  byToolUseId: Map<string, number>
  byTaskOnly: number
}

const EMPTY: TaskFeedbackIndex = {
  total: 0,
  byMessageUuid: new Map(),
  byToolUseId: new Map(),
  byTaskOnly: 0,
}

/**
 * Hook returning a derived `Map<string, number>` index keyed by `message_uuid`
 * and `tool_use_id`, used to render the "has feedback" indicator without an
 * extra query per message/tool call.
 */
export function useTaskFeedbackIndex(taskId: string | undefined): TaskFeedbackIndex {
  const { data } = useTaskFeedback(taskId)
  return (() => {
    if (!data) {
      return EMPTY
    }
    const byMessageUuid = new Map<string, number>()
    const byToolUseId = new Map<string, number>()
    let byTaskOnly = 0
    for (const item of data.items as FeedbackResponse[]) {
      if (item.toolUseId) {
        byToolUseId.set(item.toolUseId, (byToolUseId.get(item.toolUseId) ?? 0) + 1)
      } else if (item.messageUuid) {
        byMessageUuid.set(item.messageUuid, (byMessageUuid.get(item.messageUuid) ?? 0) + 1)
      } else {
        byTaskOnly += 1
      }
    }
    return {
      total: data.items.length,
      byMessageUuid,
      byToolUseId,
      byTaskOnly,
    }
  })()
}
