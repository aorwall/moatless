/**
 * Agent step functions (AGENT-01 through AGENT-05).
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type { EventStream, StreamEvent } from '../../shared/event-stream'
import type { MoatApi } from '../../shared/moat-api'
import { profile } from '../../shared/profile'
import type { GetMessagesResponse, UiMessage } from '../../shared/types'
import { waitForAgentComplete } from '../../shared/wait-helpers'

// =============================================================================
// AGENT-01: Wait for agent turn to complete
// =============================================================================

/**
 * Wait for the Nth taskTurn.completed event, falling back to API polling.
 */
export async function waitForAgentTurnComplete(
  sse: EventStream,
  api: MoatApi,
  taskId: string,
  turnNumber = 1,
  timeoutMs = profile().timeouts.agentIdle,
): Promise<{ completed: boolean; event: StreamEvent | null }> {
  const event = await sse.waitForNthEvent(
    'taskTurn',
    'completed',
    taskId,
    turnNumber,
    timeoutMs * 0.9,
  )

  if (event) {
    return { completed: true, event }
  }

  // Fallback: poll API
  const completed = await waitForAgentComplete(api, taskId, {
    timeout: timeoutMs * 0.1,
  })
  return { completed, event: null }
}

// =============================================================================
// AGENT-03 / AGENT-07: Wait for the agent to pause on a question
// =============================================================================

/** How a turn ended up: blocked on a tool call, or over. */
export interface PendingToolCall {
  /** The call the agent is waiting on an answer to, when it is waiting. */
  toolUseId: string | null
  /** Whether the turn finished instead of pausing. */
  completed: boolean
}

/**
 * The call the agent is blocked on, if it is blocked on one.
 *
 * Read from the conversation rather than from an event: `agent.pending_response`
 * is a defined event type that nothing in the platform publishes, so a test that
 * waits for it waits forever. What a paused turn does leave behind is an
 * assistant message flagged as awaiting input, carrying a tool call with no
 * result against it.
 */
export function pendingToolUseId(messages: GetMessagesResponse, toolName: string): string | null {
  const pending = [...messages.messages]
    .reverse()
    .find((m) => m.role === 'assistant' && m.hasPendingUserInput === true)
  if (!pending) {
    return null
  }
  const answered = new Set((pending.toolResults ?? []).map((r) => r.toolUseId))
  const block = [...pending.content]
    .reverse()
    .find(
      (c) =>
        c.type === 'tool_use' &&
        c.toolName === toolName &&
        c.toolUseId != null &&
        !answered.has(c.toolUseId),
    )
  return block?.toolUseId ?? null
}

/**
 * Wait until the agent is either blocked on `toolName` or done.
 *
 * Whether it asks at all is the model's decision, so both endings are answers
 * and the caller asserts on the one that happened.
 */
export async function waitForPendingToolCall(
  sse: EventStream,
  api: MoatApi,
  taskId: string,
  toolName: string,
  timeoutMs: number,
  turnsBefore = 0,
): Promise<PendingToolCall> {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    const toolUseId = pendingToolUseId(await api.getTaskMessages(taskId), toolName)
    if (toolUseId) {
      return { toolUseId, completed: false }
    }
    if (sse.countEvents('taskTurn', 'completed', taskId) > turnsBefore) {
      return { toolUseId: null, completed: true }
    }
    if (Date.now() >= deadline) {
      return { toolUseId: null, completed: false }
    }
    await new Promise((resolve) => setTimeout(resolve, 3000))
  }
}

// =============================================================================
// MSG helpers: Queued message filtering
// =============================================================================

/**
 * Return user messages that are still queued (not yet acknowledged by an agent session).
 */
export async function getQueuedMessages(api: MoatApi, taskId: string): Promise<UiMessage[]> {
  const { messages } = await api.getTaskMessages(taskId)
  return messages.filter((m) => m.isQueued === true)
}
