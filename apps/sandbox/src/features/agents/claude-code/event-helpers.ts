import type { AgentEvent, ClaudeCodeSDKMessage } from '@moatless/types'
import type { Logger } from 'pino'
import { calculateContextTokens } from './context-tokens'

/**
 * Create an AgentEvent with context token information extracted from SDK messages.
 */
export function createAgentEvent(
  payload: ClaudeCodeSDKMessage | Record<string, unknown>,
  opts: {
    agentType: string
    taskId: string
    sessionId?: string
    isError?: boolean
    subType?: string
  },
): AgentEvent {
  const contextTokens =
    'type' in payload && 'session_id' in payload
      ? calculateContextTokens(payload as ClaudeCodeSDKMessage)
      : 0

  const sessionId =
    'session_id' in payload ? (payload as ClaudeCodeSDKMessage).session_id : opts.sessionId

  return {
    eventId: crypto.randomUUID(),
    agentType: opts.agentType,
    type: 'agent',
    subType: opts.subType || ('subType' in payload ? (payload['subType'] as string) : undefined),
    taskId: opts.taskId,
    timestamp: new Date(),
    isError: opts.isError ?? false,
    payload: {
      ...payload,
      contextTokens,
      agentSessionId: sessionId,
    },
  } as AgentEvent
}

/**
 * Create a result event, optionally including an extracted plan.
 */
export function createResultEvent(
  agentLogger: Logger,
  sdkMessage: ClaudeCodeSDKMessage,
  isError: boolean,
  extractedPlan: string | undefined,
  eventOpts: {
    agentType: string
    taskId: string
    sessionId?: string
  },
): { event: AgentEvent; planConsumed: boolean } {
  if (extractedPlan) {
    agentLogger.info(
      {
        planLength: extractedPlan.length,
      },
      'Including extracted plan in result event',
    )
    const eventWithPlan = createAgentEvent(sdkMessage, {
      ...eventOpts,
      isError,
      subType: 'completed',
    })
    eventWithPlan.payload = {
      ...eventWithPlan.payload,
      result: extractedPlan,
    }
    return { event: eventWithPlan, planConsumed: true }
  }

  return {
    event: createAgentEvent(sdkMessage, {
      ...eventOpts,
      isError,
      subType: 'completed',
    }),
    planConsumed: false,
  }
}
