import type { UiContentBlock, UiMessage, UiToolResult } from '@moatless/api-client'
import { SubagentToolCall } from './subagent-tool-call.tsx'
import { ToolCallComponent } from './tool-call.tsx'

/**
 * Render tool calls list with result mapping
 */
export function renderToolCalls(
  taskId: string | undefined,
  toolCalls: UiContentBlock[],
  toolResultMap: Map<string, UiToolResult>,
  expandedToolCalls: Set<string>,
  onToggleToolCall: (toolUseId: string) => void,
  hasPendingUserInput: boolean | null | undefined,
  subagentMessages?: UiMessage[],
  parentMessageUuid?: string,
): React.ReactNode {
  if (toolCalls.length === 0) {
    return null
  }

  return (
    <div className="space-y-2 w-full max-w-full">
      {toolCalls.map((toolCall) => {
        // Render Agent tool calls with SubagentToolCall component
        if (toolCall.toolName === 'Agent' && toolCall.toolUseId) {
          return (
            <SubagentToolCall
              key={toolCall.toolUseId}
              content={toolCall}
              toolResult={toolResultMap.get(toolCall.toolUseId)}
              subagentMessages={subagentMessages}
              expanded={expandedToolCalls.has(toolCall.toolUseId)}
              onToggle={() => onToggleToolCall(toolCall.toolUseId || '')}
            />
          )
        }

        return (
          <ToolCallComponent
            key={toolCall.toolUseId}
            taskId={taskId}
            parentMessageUuid={parentMessageUuid}
            content={toolCall}
            toolResult={toolCall.toolUseId ? toolResultMap.get(toolCall.toolUseId) : undefined}
            expanded={expandedToolCalls.has(toolCall.toolUseId || '')}
            onToggle={() => onToggleToolCall(toolCall.toolUseId || '')}
            hasPendingUserInput={hasPendingUserInput}
          />
        )
      })}
    </div>
  )
}
