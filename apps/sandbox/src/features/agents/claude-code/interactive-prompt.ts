import type { AgentEvent, AskUserQuestionResponse, ExitPlanModeResponse } from '@moatless/types'
import { logger } from '../../../shared/logger'

// ===== Interactive Prompt Support =====
// Module-scoped state for when the agent pauses execution to prompt the user
// and waits for their response (AskUserQuestion, ExitPlanMode).

type PendingInteractivePrompt = {
  toolName: 'AskUserQuestion' | 'ExitPlanMode'
  resolve: (response: AskUserQuestionResponse | ExitPlanModeResponse) => void
  reject: (error: Error) => void
}

let pendingPrompt: PendingInteractivePrompt | null = null

/**
 * Resolve the pending interactive prompt with the provided response.
 * Called by the agent routes when user submits their answer.
 */
export function resolveInteractivePrompt(
  response: AskUserQuestionResponse | ExitPlanModeResponse,
): boolean {
  if (pendingPrompt) {
    pendingPrompt.resolve(response)
    pendingPrompt = null
    return true
  }
  return false
}

/**
 * Check if there is an active interactive prompt
 */
export function hasActivePrompt(): boolean {
  return pendingPrompt !== null
}

/**
 * Get the tool name of the active interactive prompt
 */
export function getActivePromptToolName(): 'AskUserQuestion' | 'ExitPlanMode' | null {
  return pendingPrompt?.toolName ?? null
}

/**
 * Handle interactive prompt tools (AskUserQuestion, ExitPlanMode).
 * Pauses execution until user provides input via the API endpoint.
 */
export async function handleInteractivePrompt(
  toolName: 'AskUserQuestion' | 'ExitPlanMode',
  input: Record<string, unknown>,
  taskId: string,
): Promise<
  | { behavior: 'allow'; updatedInput: Record<string, unknown> }
  | { behavior: 'deny'; message: string }
> {
  const promptLogger = logger.child({ taskId, toolName })

  promptLogger.info({ input }, `${toolName} tool invoked, waiting for user input`)

  // Create promise that will be resolved when response arrives
  // No timeout - wait indefinitely for user input
  const responsePromise = new Promise<AskUserQuestionResponse | ExitPlanModeResponse>((resolve) => {
    // No reject needed - we wait indefinitely for user input
    pendingPrompt = { toolName, resolve, reject: () => undefined }
  })

  // Wait for response (tool_use message already flows to frontend via SSE)
  const response = await responsePromise

  promptLogger.info({ toolName, response }, 'User input received')

  // Handle response based on tool type
  if (toolName === 'AskUserQuestion') {
    const answerResponse = response as AskUserQuestionResponse
    return {
      behavior: 'allow',
      updatedInput: {
        ...input,
        answers: answerResponse.answers,
      },
    }
  }

  if (toolName === 'ExitPlanMode') {
    const planResponse = response as ExitPlanModeResponse
    if (planResponse.approved) {
      return {
        behavior: 'allow',
        updatedInput: input,
      }
    }
    return {
      behavior: 'deny',
      message: planResponse.feedback || 'User rejected the plan',
    }
  }

  return { behavior: 'allow', updatedInput: input }
}

/**
 * Emit a pending_response event to notify channels that the agent is waiting for user input.
 * This is emitted BEFORE the agent blocks waiting for response, allowing Slack/Email to prompt user.
 */
export function emitPendingResponseEvent(
  toolName: 'AskUserQuestion' | 'ExitPlanMode',
  toolInput: Record<string, unknown>,
  opts: {
    taskId: string
    agentType: string
    sessionId: string | undefined
    emitEvent: (event: AgentEvent) => void
  },
): void {
  const promptLogger = logger.child({ taskId: opts.taskId, toolName })
  promptLogger.info('Emitting pending_response event for interactive tool')

  opts.emitEvent({
    eventId: crypto.randomUUID(),
    agentType: opts.agentType,
    type: 'agent',
    subType: 'pending_response',
    taskId: opts.taskId,
    timestamp: new Date(),
    isError: false,
    payload: {
      toolName,
      toolInput,
      agentSessionId: opts.sessionId,
    },
  } as AgentEvent)
}
