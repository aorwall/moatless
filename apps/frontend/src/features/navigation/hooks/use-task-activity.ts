import { useEffect, useState } from 'react'
import { useGlobalEventStream } from '@/features/events/contexts/global-event-stream-provider'
import type { TaskActivityState } from '@/features/tasks/utils/task-status'
import { isTextContent, isToolUseContent } from '@/lib/types/content-guards'
import type { Event } from '@/lib/types/events'
import { formatPreview } from './message-preview'

/**
 * Live, per-task activity derived from the global event stream.
 *
 * The task list API seeds `agentState`/`isRunning`/`environmentStatus` for list
 * rows (a last-known, eventually-consistent snapshot), but the live message
 * preview and up-to-the-second running state still come from the SSE stream.
 * Entries accumulate as events arrive and are merged with the cached task list
 * at render time, with live activity taking precedence over the seeded row
 * state.
 */
/** How many recent message previews to keep for the live running view. */
export const MAX_RECENT_MESSAGES = 3

export interface TaskActivity {
  /** Whether the agent is currently producing output for this task. */
  agentWorking: boolean
  /**
   * Latest observed sandbox phase from the stream: `initializing` while the
   * sandbox spins up, `ready` once it is up, or `null` when it is down / never
   * observed.
   */
  sandboxPhase: 'initializing' | 'ready' | null
  /** Short text preview of the most recent message, if one has streamed in. */
  lastMessage: string | null
  /** ISO timestamp of the most recent streamed message. */
  lastMessageAt: string | null
  /**
   * The last few streamed message previews (oldest-first, up to
   * {@link MAX_RECENT_MESSAGES}). Used to fill running rows with a few lines of
   * live activity, mirroring the task dashboard.
   */
  recentMessages: string[]
}

/**
 * Collapse a task's live activity into a single display state. The agent
 * actively working takes precedence, then a starting sandbox, then a ready
 * (but idle) sandbox; anything else is idle.
 */
export function activityToState(activity: TaskActivity | undefined): TaskActivityState {
  if (!activity) {
    return 'idle'
  }
  if (activity.agentWorking) {
    return 'agent-working'
  }
  if (activity.sandboxPhase === 'initializing') {
    return 'starting'
  }
  if (activity.sandboxPhase === 'ready') {
    return 'sandbox-running'
  }
  return 'idle'
}

/**
 * Extract a human-readable preview from an agent `message` event payload.
 *
 * Agent payloads are pass-through Claude SDK messages: `payload.message` is the
 * assistant/user message whose `content` is either a plain string or an array
 * of typed blocks (`text`, `tool_use`, …). Returns `null` when there is nothing
 * worth showing so the previous preview is preserved.
 */
export function extractMessagePreview(event: Event): string | null {
  if (event.type !== 'agent' || event.subType !== 'message') {
    return null
  }

  const message = event.payload?.message
  if (!message || typeof message !== 'object') {
    return null
  }

  const content = (message as { content?: unknown }).content

  if (typeof content === 'string') {
    const preview = formatPreview(content)
    return preview.length > 0 ? preview : null
  }

  if (!Array.isArray(content)) {
    return null
  }

  const texts: string[] = []
  let toolName: string | null = null

  for (const block of content) {
    if (isTextContent(block)) {
      texts.push(block.text)
    } else if (isToolUseContent(block) && !toolName) {
      toolName = block.name
    }
  }

  const joined = texts.join(' ').trim()
  if (joined.length > 0) {
    return formatPreview(joined)
  }

  // No prose in this message — surface the tool the agent reached for instead.
  return toolName ? `Using ${toolName}…` : null
}

/** Whether an event marks the agent as having started working on a task. */
function isRunningStart(event: Event): boolean {
  if (event.type === 'agent') {
    return event.subType === 'started' || event.subType === 'message'
  }
  if (event.type === 'taskTurn') {
    return event.subType === 'active'
  }
  return false
}

/** Whether an event marks the agent as having stopped working on a task. */
function isRunningStop(event: Event): boolean {
  if (event.type === 'agent') {
    return event.subType === 'completed' || event.subType === 'stopped' || event.subType === 'error'
  }
  if (event.type === 'taskTurn') {
    return event.subType === 'completed' || event.subType === 'error'
  }
  return false
}

/**
 * Derive the next sandbox phase from an event. Sandbox lifecycle events set the
 * phase directly; any agent activity implies a ready sandbox, so a task keeps
 * showing as "sandbox running" once its agent finishes (until the sandbox is
 * later reported stopped/removed).
 */
function nextSandboxPhase(
  event: Event,
  current: 'initializing' | 'ready' | null,
): 'initializing' | 'ready' | null {
  if (event.type === 'sandbox') {
    switch (event.subType) {
      case 'initializing':
        return 'initializing'
      case 'ready':
        return 'ready'
      case 'stopped':
      case 'removing':
      case 'removed':
      case 'not_created':
      case 'error':
        return null
      default:
        return current
    }
  }
  // An agent can only run against a ready sandbox — infer readiness so the row
  // stays in the running section as "sandbox running" after the agent stops.
  if (isRunningStart(event)) {
    return 'ready'
  }
  return current
}

/** Whether an event can affect a task's activity, beyond message previews. */
function affectsActivity(event: Event): boolean {
  return isRunningStart(event) || isRunningStop(event) || event.type === 'sandbox'
}

/**
 * Subscribe to the global event stream and maintain a live activity map keyed
 * by task id. Only produces a new map (and therefore a re-render) when an event
 * actually changes a task's running state or message preview.
 */
export function useTaskActivity(): Map<string, TaskActivity> {
  const { subscribeToAll } = useGlobalEventStream()
  const [activity, setActivity] = useState<Map<string, TaskActivity>>(() => new Map())

  useEffect(() => {
    return subscribeToAll((event) => {
      const taskId = event.taskId
      if (!taskId) {
        return
      }

      const preview = extractMessagePreview(event)

      if (!affectsActivity(event) && preview === null) {
        return
      }

      setActivity((prev) => {
        const current = prev.get(taskId) ?? {
          agentWorking: false,
          sandboxPhase: null,
          lastMessage: null,
          lastMessageAt: null,
          recentMessages: [],
        }

        const agentWorking = isRunningStop(event)
          ? false
          : isRunningStart(event) || current.agentWorking

        const next: TaskActivity = {
          agentWorking,
          sandboxPhase: nextSandboxPhase(event, current.sandboxPhase),
          lastMessage: preview ?? current.lastMessage,
          lastMessageAt: preview ? event.timestamp : current.lastMessageAt,
          // Keep a rolling window of the last few previews so running rows can
          // show several lines of recent activity.
          recentMessages: preview
            ? [...current.recentMessages, preview].slice(-MAX_RECENT_MESSAGES)
            : current.recentMessages,
        }

        if (
          next.agentWorking === current.agentWorking &&
          next.sandboxPhase === current.sandboxPhase &&
          next.lastMessage === current.lastMessage &&
          next.lastMessageAt === current.lastMessageAt
        ) {
          return prev
        }

        const updated = new Map(prev)
        updated.set(taskId, next)
        return updated
      })
    })
  }, [subscribeToAll])

  return activity
}
