import type { UiContentBlock, UiToolResult } from '@moatless/api-client'

export interface ToolCallProps {
  taskId?: string | undefined
  /** UUID of the assistant message this tool call belongs to. */
  parentMessageUuid?: string | undefined
  content: UiContentBlock
  toolResult?: UiToolResult | undefined
  expanded: boolean
  onToggle: () => void
  hasPendingUserInput?: boolean | null | undefined
}
