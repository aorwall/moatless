import { useState } from 'react'

/**
 * Persists the chat input draft per task in localStorage so text survives
 * page reloads and switching between tasks without being lost.
 *
 * The draft is keyed by task id. Switching tasks loads that task's draft;
 * clearing the input (e.g. after a successful send) removes the stored draft.
 */
const DRAFT_KEY_PREFIX = 'taskDraftMessage:'

function draftKey(taskId: string): string {
  return `${DRAFT_KEY_PREFIX}${taskId}`
}

function readDraft(taskId: string): string {
  if (typeof window === 'undefined' || !taskId) {
    return ''
  }
  try {
    return localStorage.getItem(draftKey(taskId)) ?? ''
  } catch (_error) {
    // localStorage.getItem may throw in restricted contexts
    return ''
  }
}

function writeDraft(taskId: string, value: string): void {
  if (typeof window === 'undefined' || !taskId) {
    return
  }
  try {
    if (value) {
      localStorage.setItem(draftKey(taskId), value)
    } else {
      // Keep storage clean: an empty draft is the same as no draft
      localStorage.removeItem(draftKey(taskId))
    }
  } catch (_error) {
    // localStorage.setItem may throw in private browsing mode / when over quota
  }
}

/**
 * A per-task draft message backed by localStorage.
 *
 * @returns A tuple of the current draft and a setter that also persists it
 */
export function useTaskDraftMessage(taskId: string): [string, (value: string) => void] {
  const [message, setMessageState] = useState<string>(() => readDraft(taskId))
  const [prevTaskId, setPrevTaskId] = useState(taskId)

  // When the task changes, load that task's persisted draft. This is React's
  // recommended "adjust state during render" pattern, avoiding an effect.
  if (taskId !== prevTaskId) {
    setPrevTaskId(taskId)
    setMessageState(readDraft(taskId))
  }

  const setMessage = (value: string) => {
    setMessageState(value)
    writeDraft(taskId, value)
  }

  return [message, setMessage]
}
