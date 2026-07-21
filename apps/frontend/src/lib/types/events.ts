/**
 * Discriminated event sub-types built on top of the api-client's StreamEvent.
 * The backend sends generic StreamEvent; the frontend narrows by `type` field.
 */
import type {
  AgentEventPayload,
  AgentSubType,
  SandboxStatus,
  ServerEventPayload,
  ServerSubType,
  StreamEvent,
  TaskSubType,
  TaskTurnSubType,
  UiMessage,
} from '@moatless/api-client/sandbox/model'

/** Derived, UI-only message-event subtypes (not in the generated model). */
export type MessageSubType = 'upsert' | 'deleted' | 'subagentProgress'

export interface TaskEvent extends StreamEvent {
  type: 'task'
  subType: TaskSubType
}

export interface TaskTurnEvent extends StreamEvent {
  type: 'taskTurn'
  subType: TaskTurnSubType
}

export interface SandboxEvent extends StreamEvent {
  type: 'sandbox'
  subType: SandboxStatus
}

export interface ServerEvent extends StreamEvent {
  type: 'server'
  subType: ServerSubType
  payload: ServerEventPayload
}

export interface ResourcesEvent extends StreamEvent {
  type: 'resources'
}

export interface AgentEvent extends StreamEvent {
  type: 'agent'
  subType: AgentSubType
  payload: AgentEventPayload
}

/**
 * Derived, UI-only event carrying a server-assembled Message (ADR-0002).
 * `upsert` payload is the assembled `UiMessage`; `deleted` payload is its uuid.
 */
export interface MessageUpsertEvent extends StreamEvent {
  type: 'message'
  subType: 'upsert'
  payload: UiMessage
}

export interface MessageDeletedEvent extends StreamEvent {
  type: 'message'
  subType: 'deleted'
  payload: { uuid: string }
}

/**
 * Derived, UI-only event carrying one subagent's absolute activity summary
 * (ADR-0003 lazy subagents). Written through onto the cached parent Message's
 * `Agent` block; no subagent transcript content ever rides the wire.
 */
export interface MessageSubagentProgressEvent extends StreamEvent {
  type: 'message'
  subType: 'subagentProgress'
  payload: {
    toolUseId: string
    messageCount: number
    tokens: number
    status: 'running' | 'completed' | 'error'
  }
}

export type MessageEvent = MessageUpsertEvent | MessageDeletedEvent | MessageSubagentProgressEvent

export interface NavigationEvent extends Omit<StreamEvent, 'type'> {
  type: 'navigation'
}

export interface PreviewEvent extends Omit<StreamEvent, 'type'> {
  type: 'preview'
}

/**
 * Discriminated union of all specific event types.
 * Use this instead of StreamEvent when you need to narrow on event.type.
 */
export type Event =
  | TaskEvent
  | TaskTurnEvent
  | SandboxEvent
  | ServerEvent
  | ResourcesEvent
  | AgentEvent
  | MessageUpsertEvent
  | MessageDeletedEvent
  | MessageSubagentProgressEvent
  | NavigationEvent
  | PreviewEvent

/**
 * Extended EventType that includes frontend-only event types not in the backend enum.
 */
export type EventType =
  | import('@moatless/api-client/sandbox/model').EventType
  | 'navigation'
  | 'preview'

// Re-export StreamEvent and AgentEventPayload from api-client for convenience
export type { AgentEventPayload, StreamEvent } from '@moatless/api-client/sandbox/model'
