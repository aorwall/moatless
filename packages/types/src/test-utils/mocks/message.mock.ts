import type {
  AssistantMessage,
  AssistantMessageContent,
  Message,
  MessagesResponse,
  ToolResult,
  UserMessage,
} from '../../schemas/message.schemas.js'
import type { ToolCallGroup } from '../../types/message.js'

export function createMockMessage(overrides?: Partial<UserMessage>): UserMessage {
  return createMockUserMessage('Test message', overrides)
}

export function createMockUserMessage(
  content: string,
  overrides?: Partial<UserMessage>,
): UserMessage {
  return {
    id: 'user-msg-123',
    role: 'user',
    content,
    timestamp: new Date('2024-01-01T00:00:00Z'),
    eventId: 'event-123',
    ...overrides,
  }
}

export function createMockAssistantMessage(
  content: string | AssistantMessageContent[],
  overrides?: Partial<AssistantMessage>,
): AssistantMessage {
  return {
    id: 'assistant-msg-123',
    role: 'assistant',
    content,
    timestamp: new Date('2024-01-01T00:01:00Z'),
    eventId: 'event-124',
    ...overrides,
  }
}

export function createMockTextContent(text: string): AssistantMessageContent {
  return {
    type: 'text',
    text,
  }
}

export function createMockToolUseContent(
  toolName: string,
  toolInput: Record<string, unknown>,
): AssistantMessageContent {
  return {
    type: 'tool_use',
    tool_name: toolName,
    tool_input: toolInput,
    tool_use_id: `tool-use-${Math.random().toString(36).substr(2, 9)}`,
  }
}

export function createMockToolResult(content: string, isError = false): ToolResult {
  return {
    content,
    is_error: isError,
    eventId: `event-${Math.random().toString(36).substr(2, 9)}`,
  }
}

export function createMockMessageWithToolUse(
  overrides?: Partial<AssistantMessage>,
): AssistantMessage {
  const toolUseContent = createMockToolUseContent('read_file', { file_path: '/test/file.ts' })
  const textContent = createMockTextContent('Let me read the file for you.')

  return createMockAssistantMessage([textContent, toolUseContent], overrides)
}

export function createMockMessageWithToolResult(
  overrides?: Partial<AssistantMessage>,
): AssistantMessage {
  const toolUseContent = createMockToolUseContent('read_file', { file_path: '/test/file.ts' })
  toolUseContent.tool_result = createMockToolResult('File contents here')

  const textContent = createMockTextContent('I have read the file.')

  return createMockAssistantMessage([textContent, toolUseContent], overrides)
}

export function createMockMessagesResponse(messageCount = 5): MessagesResponse {
  const messages: Message[] = []

  for (let i = 0; i < messageCount; i++) {
    if (i % 2 === 0) {
      messages.push(
        createMockUserMessage(`User message ${i + 1}`, {
          id: `user-msg-${i + 1}`,
          timestamp: new Date(`2024-01-01T00:${String(i).padStart(2, '0')}:00Z`),
        }),
      )
    } else {
      messages.push(
        createMockAssistantMessage(`Assistant response ${i + 1}`, {
          id: `assistant-msg-${i + 1}`,
          timestamp: new Date(`2024-01-01T00:${String(i).padStart(2, '0')}:30Z`),
        }),
      )
    }
  }

  return {
    messages,
    totalCount: messageCount,
  }
}

export function createMockToolCallGroup(toolName = 'read_file'): ToolCallGroup {
  return {
    toolUseEvent: {
      id: 'tool-use-event-123',
      type: 'message',
      timestamp: new Date('2024-01-01T00:00:00Z'),
      content: {
        type: 'tool_use',
        tool_name: toolName,
        tool_input: { file_path: '/test/file.ts' },
        tool_use_id: 'tool-use-123',
      },
    } as any,
    toolResultEvent: {
      id: 'tool-result-event-123',
      type: 'tool_result',
      timestamp: new Date('2024-01-01T00:00:01Z'),
      content: {
        tool_use_id: 'tool-use-123',
        content: 'File contents here',
        is_error: false,
      },
    } as any,
    toolUseId: 'tool-use-123',
  }
}
