import type { Event } from '@shared/types/event'
import { ChevronDown, ChevronRight } from 'lucide-react'
import type { ReactNode } from 'react'
import { useState } from 'react'
import type { MergedEvent } from '@/features/tasks/utils/merge-tool-events'

import {
  BashToolCall,
  EditToolCall,
  GlobToolCall,
  GrepToolCall,
  LSToolCall,
  ReadToolCall,
  TaskToolCall,
  TodoToolCall,
  WriteToolCall,
} from './tool-calls'

interface ToolCallEventProps {
  event: MergedEvent
}

const renderToolCall = (content: Record<string, unknown>): ReactNode => {
  const toolName = String(content.name || 'Tool')
  const toolId = String(content.id || '')
  const input = content.input as Record<string, unknown> | undefined

  if (!input || typeof input !== 'object') {
    return (
      <div className="space-y-2">
        <div className="font-medium text-foreground">{toolName}</div>
        <div className="text-xs text-muted-foreground">ID: {toolId}</div>
      </div>
    )
  }

  // Use existing tool call components
  switch (toolName) {
    case 'Bash':
      return (
        <BashToolCall
          input={
            input as {
              command: string
              description?: string
              timeout?: number
            }
          }
          name={toolName}
        />
      )
    case 'Edit':
      return (
        <EditToolCall
          input={
            input as {
              filePath: string
              oldString: string
              newString: string
              replaceAll?: boolean
            }
          }
          name={toolName}
        />
      )
    case 'Read':
      return (
        <ReadToolCall
          input={
            input as {
              filePath: string
              offset?: number
              limit?: number
            }
          }
          name={toolName}
        />
      )
    case 'TodoWrite':
      return (
        <TodoToolCall
          input={
            input as {
              todos: Array<{
                id: string
                content: string
                status: 'pending' | 'inProgress' | 'completed'
                priority: 'high' | 'medium' | 'low'
              }>
            }
          }
          name={toolName}
        />
      )
    case 'Grep':
      return (
        <GrepToolCall
          input={
            input as {
              pattern: string
              path?: string
              include?: string
            }
          }
          name={toolName}
        />
      )
    case 'Glob':
      return (
        <GlobToolCall
          input={
            input as {
              pattern: string
              path?: string
            }
          }
          name={toolName}
        />
      )
    case 'Write':
      return (
        <WriteToolCall
          input={
            input as {
              filePath: string
              content: string
            }
          }
          name={toolName}
        />
      )
    case 'LS':
      return (
        <LSToolCall
          input={
            input as {
              path: string
              ignore?: string[]
            }
          }
          name={toolName}
        />
      )
    case 'Task':
      return (
        <TaskToolCall
          input={
            input as {
              description: string
              prompt: string
            }
          }
          name={toolName}
        />
      )
    default:
      return (
        <div className="space-y-2">
          <div className="font-medium text-foreground">{toolName}</div>
          <div className="text-xs bg-muted rounded p-2 overflow-x-auto">
            <pre className="font-mono">{JSON.stringify(input, null, 2)}</pre>
          </div>
        </div>
      )
  }
}

const extractToolResult = (event: Event): string => {
  const payload = event.payload as Record<string, unknown>
  const message = payload.message as Record<string, unknown> | null

  if (!(message?.content && Array.isArray(message.content))) {
    return 'No result'
  }

  for (const content of message.content) {
    if (
      content &&
      typeof content === 'object' &&
      'type' in content &&
      content.type === 'toolResult' &&
      'content' in content
    ) {
      return String(content.content || 'No result')
    }
  }

  return 'No result'
}

const ToolResultPreview = ({ result }: { result: string }) => {
  const [isExpanded, setIsExpanded] = useState(false)

  const lines = result.split('\n')
  const hasMoreLines = lines.length > 3
  const previewLines = lines.slice(0, 3).join('\n')
  const displayContent = isExpanded ? result : previewLines

  return (
    <div className="mt-3 space-y-2">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span>Tool Result</span>
        {hasMoreLines && (
          <button
            onClick={() => setIsExpanded(!isExpanded)}
            className="flex items-center gap-1 hover:text-foreground transition-colors"
          >
            {isExpanded ? (
              <ChevronDown className="h-3 w-3" />
            ) : (
              <ChevronRight className="h-3 w-3" />
            )}
            <span>{isExpanded ? 'Collapse' : 'Expand'}</span>
          </button>
        )}
      </div>
      <div className="text-xs bg-zinc-800 dark:bg-zinc-900 text-zinc-300 dark:text-zinc-400 rounded p-3 overflow-x-auto">
        <pre className="whitespace-pre-wrap break-words font-mono">
          {displayContent}
          {!isExpanded && hasMoreLines && (
            <button
              onClick={() => setIsExpanded(true)}
              className="text-zinc-500 hover:text-zinc-300 dark:text-zinc-600 dark:hover:text-zinc-400 transition-colors cursor-pointer"
            >
              {'\n'}... ({lines.length - 3} more lines)
            </button>
          )}
        </pre>
      </div>
    </div>
  )
}

export function ToolCallEvent({ event }: ToolCallEventProps) {
  const payload = event.payload as Record<string, unknown>
  const message = payload.message as Record<string, unknown> | null

  if (!message) {
    return <div className="text-sm text-muted-foreground">No message content</div>
  }

  // Extract tool call content
  let toolCallContent: Record<string, unknown> | null = null

  if (message.content && Array.isArray(message.content)) {
    for (const content of message.content) {
      if (
        content &&
        typeof content === 'object' &&
        'type' in content &&
        content.type === 'toolUse'
      ) {
        toolCallContent = content as Record<string, unknown>
        break
      }
    }
  }

  if (!toolCallContent) {
    return <div className="text-sm text-muted-foreground">No tool call content</div>
  }

  const toolResult = event.toolResult ? extractToolResult(event.toolResult) : null

  return (
    <div className="text-sm bg-muted rounded-lg p-3">
      {renderToolCall(toolCallContent)}
      {toolResult && <ToolResultPreview result={toolResult} />}
    </div>
  )
}
