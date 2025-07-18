// Re-export all event types from event schemas
export type {
  BaseEvent,
  ErrorEvent,
  Event,
  EventPayload,
  EventType,
  MessageEvent,
  ResultEvent,
  ServerEvent,
  SpecificEvent,
  TaskStatusEvent,
  WorkspaceEvent,
} from '../schemas/event.schemas.js'

// Re-export claude types from their original source
export type { ClaudeCodeSDKMessage, SDKResultMessage } from './claude.js'
