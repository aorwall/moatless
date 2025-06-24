import type { SDKMessage } from '@anthropic-ai/claude-code'

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
}

export enum MessageSubType {
  TOOL_CALL = 'tool_call',
  TOOL_RESULT = 'tool_result',
  TEXT = 'text',
}

export interface BaseEvent {
  eventId: string
  type: EventType
  subType?: string
  taskId: string
  iterationId?: number
  timestamp: Date
  isError: boolean
  contextTokens?: number
  numTurns?: number
  totalCost?: number
}

export interface EventPayload {
  [key: string]: any
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
  payload: SDKMessage
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
  subType:
    | 'setup_started'
    | 'setup_completed'
    | 'setup_failed'
    | 'worktree_created'
    | 'directory_created'
    | 'command_running'
    | 'command_completed'
    | 'command_failed'
  payload: {
    workspaceId?: string
    error?: string
  }
}

export interface ServerEvent extends BaseEvent {
  type: EventType.SERVER
  subType:
    | 'server_starting'
    | 'server_started'
    | 'server_stopping'
    | 'server_stopped'
    | 'server_failed'
    | 'server_start_failed'
  payload: {
    port?: number
    url?: string
    pid?: number
    error?: string
  }
}

export interface MessageContent {
  type: string
  text?: string
  id?: string
  name?: string
  input?: Record<string, any>
  toolUseId?: string
  content?: string
  isError?: boolean
}

export type SpecificEvent = InputEvent | MessageEvent | ErrorEvent | WorkspaceEvent | ServerEvent
