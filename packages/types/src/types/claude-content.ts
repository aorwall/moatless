// Type definitions for Claude SDK message content
// These are inferred from the actual SDK usage patterns

import type { MoatlessToolInput, MoatlessToolName } from './tools.js'

export interface TextContent {
  type: 'text'
  text: string
}

/**
 * Generic tool use content - name is string to support all tools (Claude SDK, user MCP servers, etc.)
 */
export interface ToolUseContent {
  type: 'tool_use'
  id: string
  name: string
  input: Record<string, unknown>
}

/**
 * Typed tool use content for Moatless tools with type-safe name and input
 */
export interface MoatlessToolUseContent<T extends MoatlessToolName = MoatlessToolName> {
  type: 'tool_use'
  id: string
  name: T
  input: MoatlessToolInput<T>
}

export interface ToolResultContent {
  type: 'tool_result'
  tool_use_id: string
  content: string
  is_error?: boolean
}

export interface ImageContentBlock {
  type: 'image'
  source: {
    type: 'base64'
    media_type: 'image/png' | 'image/jpeg' | 'image/gif' | 'image/webp'
    data: string
  }
}

export type AssistantContent = TextContent | ToolUseContent
export type UserContent = TextContent | ToolResultContent | ImageContentBlock

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

export function isImageContentBlock(content: unknown): content is ImageContentBlock {
  return (
    content !== null &&
    typeof content === 'object' &&
    'type' in content &&
    content.type === 'image' &&
    'source' in content &&
    typeof content.source === 'object' &&
    content.source !== null &&
    'type' in content.source &&
    content.source.type === 'base64' &&
    'media_type' in content.source &&
    'data' in content.source
  )
}

/**
 * Check if content array contains any tools
 */
export function hasTools(content: unknown[]): boolean {
  if (!Array.isArray(content)) {
    return false
  }

  return content.some((item) => isToolUseContent(item))
}

/**
 * Extract all tool names from content array
 */
export function extractToolNames(content: unknown[]): string[] {
  if (!Array.isArray(content)) {
    return []
  }

  return content.filter(isToolUseContent).map((item) => item.name)
}
