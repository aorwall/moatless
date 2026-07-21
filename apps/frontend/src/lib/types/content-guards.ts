/**
 * Runtime type guard functions for Claude SDK ContentBlock variants.
 *
 * The actual ContentBlock union type comes from @moatless/api-client/sandbox/model.
 * These guards narrow unknown payloads to specific variants at runtime.
 */
import type { ContentBlock } from '@moatless/api-client/sandbox/model'

// Narrowed variant types extracted from the ContentBlock discriminated union
export type TextContent = Extract<ContentBlock, { type: 'text' }>
export type ToolUseContent = Extract<ContentBlock, { type: 'tool_use' }>
export type ToolResultContent = Extract<ContentBlock, { type: 'tool_result' }>
export type ImageContentBlock = Extract<ContentBlock, { type: 'image' }>

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

export function hasTools(content: unknown[]): boolean {
  if (!Array.isArray(content)) {
    return false
  }

  return content.some((item) => isToolUseContent(item))
}

export function extractToolNames(content: unknown[]): string[] {
  if (!Array.isArray(content)) {
    return []
  }

  const toolNames: string[] = []
  for (const item of content) {
    if (isToolUseContent(item)) {
      toolNames.push(item.name)
    }
  }
  return toolNames
}
