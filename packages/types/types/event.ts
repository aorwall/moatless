import type { ClaudeCodeSDKMessage, SDKResultMessage } from './claude.js'

export enum EventType {
  INPUT = 'input',
  MESSAGE = 'message',
  START = 'start',
  STOP = 'stop',
  RESULT = 'result',
  ERROR = 'error',
  WORKSPACE = 'workspace',
  SERVER = 'server',
  TOOL = 'tool',
  COMPLETION = 'completion',
  TASK_STATUS = 'task_status',
}

export interface BaseEvent {
  eventId: string
  agentType: string
  type: EventType
  subType?: string
  taskId: string
  iterationId?: number
  timestamp?: Date
  isError: boolean
  contextTokens?: number
  numTurns?: number
  totalCost?: number
}

export interface EventPayload {
  [key: string]: unknown
}

export interface Event extends BaseEvent {
  payload: EventPayload
}

export interface InputEvent extends BaseEvent {
  type: EventType.INPUT
  payload: {
    message: string
  }
}

export interface MessageEvent extends BaseEvent {
  type: EventType.MESSAGE
  agentSessionId?: string
  payload: ClaudeCodeSDKMessage
}

export interface ErrorEvent extends BaseEvent {
  type: EventType.ERROR
  isError: true
  payload: {
    error: string
    stackTrace?: string
    code?: string
  }
}

export interface WorkspaceEvent extends BaseEvent {
  type: EventType.WORKSPACE
  payload: EventPayload
}

export interface ServerEvent extends BaseEvent {
  type: EventType.SERVER
  payload: EventPayload
}

export interface ResultEvent extends BaseEvent {
  type: EventType.RESULT
  payload: SDKResultMessage
}

export interface TaskStatusEvent extends BaseEvent {
  type: EventType.TASK_STATUS
  subType: string // The status that was changed to (e.g., 'open', 'running', 'completed')
  payload: {
    status: string
    previousStatus: string
    iterationNumber?: number
  }
}

export type SpecificEvent =
  | InputEvent
  | MessageEvent
  | ErrorEvent
  | WorkspaceEvent
  | ServerEvent
  | ResultEvent
  | TaskStatusEvent
