import type { ClaudeCodeSDKMessage } from '@shared/types/claude.js'

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

export type SpecificEvent = InputEvent | MessageEvent | ErrorEvent | WorkspaceEvent | ServerEvent
