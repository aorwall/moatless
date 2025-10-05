// Re-export types from the schema file
export type {
  AgentMessage,
  AgentMessagesResponse,
  AssistantAgentMessage,
  ToolUseResult,
  UserAgentMessage,
} from '../schemas/agent-message.schemas.js'

// Import types for use in type guards
import type {
  AgentMessage,
  AssistantAgentMessage,
  UserAgentMessage,
} from '../schemas/agent-message.schemas.js'

// Type guards for agent messages
export function isUserAgentMessage(message: unknown): message is UserAgentMessage {
  return (
    message !== null &&
    typeof message === 'object' &&
    'type' in message &&
    message.type === 'user' &&
    'message' in message &&
    typeof (message as Record<string, unknown>)['message'] === 'object' &&
    (message as Record<string, unknown>)['message'] !== null &&
    typeof (message as Record<string, unknown>)['message'] === 'object' &&
    'role' in ((message as Record<string, unknown>)['message'] as Record<string, unknown>) &&
    ((message as Record<string, unknown>)['message'] as Record<string, unknown>)['role'] === 'user'
  )
}

export function isAssistantAgentMessage(message: unknown): message is AssistantAgentMessage {
  return (
    message !== null &&
    typeof message === 'object' &&
    'type' in message &&
    message.type === 'assistant' &&
    'message' in message &&
    typeof (message as Record<string, unknown>)['message'] === 'object' &&
    (message as Record<string, unknown>)['message'] !== null &&
    'role' in ((message as Record<string, unknown>)['message'] as Record<string, unknown>) &&
    ((message as Record<string, unknown>)['message'] as Record<string, unknown>)['role'] ===
      'assistant'
  )
}

// Type guard to check if an object is an AgentMessage
export function isAgentMessage(obj: unknown): obj is AgentMessage {
  if (!obj || typeof obj !== 'object') return false

  const candidate = obj as Record<string, unknown>

  // Check required fields
  if (
    typeof candidate['uuid'] !== 'string' ||
    typeof candidate['timestamp'] !== 'string' ||
    typeof candidate['type'] !== 'string' ||
    !candidate['message'] ||
    typeof candidate['message'] !== 'object'
  ) {
    return false
  }

  // Check optional fields if they exist
  if (candidate['sessionId'] !== undefined && typeof candidate['sessionId'] !== 'string') {
    return false
  }
  if (candidate['cwd'] !== undefined && typeof candidate['cwd'] !== 'string') {
    return false
  }
  if (candidate['version'] !== undefined && typeof candidate['version'] !== 'string') {
    return false
  }

  // Check if it's either a user or assistant message
  return isUserAgentMessage(obj) || isAssistantAgentMessage(obj)
}
