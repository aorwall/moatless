// Type definitions for Claude SDK message content
// These are inferred from the actual SDK usage patterns

export interface TextContent {
  type: 'text'
  text: string
}

export interface ToolUseContent {
  type: 'tool_use'
  id: string
  name: string
  input: Record<string, unknown>
}

export interface ToolResultContent {
  type: 'tool_result'
  tool_use_id: string
  content: string
}

export type AssistantContent = TextContent | ToolUseContent
export type UserContent = TextContent | ToolResultContent

// Type guards
export function isTextContent(content: unknown): content is TextContent {
  return (
    content !== null &&
    typeof content === 'object' &&
    'type' in content &&
    content.type === 'text' &&
    'text' in content &&
    typeof content.text === 'string'
  )
}

export function isToolUseContent(content: unknown): content is ToolUseContent {
  return (
    content !== null &&
    typeof content === 'object' &&
    'type' in content &&
    content.type === 'tool_use' &&
    'id' in content &&
    'name' in content &&
    'input' in content
  )
}

export function isToolResultContent(content: unknown): content is ToolResultContent {
  return (
    content !== null &&
    typeof content === 'object' &&
    'type' in content &&
    content.type === 'tool_result' &&
    'tool_use_id' in content &&
    'content' in content
  )
}
