// Re-export all event types from event schemas
export type {
  BaseEvent,
  ErrorEvent,
  Event,
  EventListResponse,
  EventPayload,
  EventType,
  MessageEvent,
  ResultEvent,
  ServerEvent,
  SpecificEvent,
  StreamStatusResponse,
  TaskStatusEvent,
  WorkspaceEvent,
} from '../schemas/event.schemas.js'

// Re-export claude types from their original source
export type { ClaudeCodeSDKMessage, SDKResultMessage } from './claude.js'
