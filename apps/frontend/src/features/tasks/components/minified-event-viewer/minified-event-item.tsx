import { isToolResultContent, isToolUseContent, type ToolUseContent } from '@moatless/shared'
import type { Event, MessageEvent } from '@moatless/shared/types/event'
import {
  Bot,
  Code2,
  Edit,
  FileEdit,
  FileSearch,
  FileText,
  Globe,
  ListTodo,
  Plus,
  Search,
  Terminal,
} from 'lucide-react'
import type { MergedEvent } from '@/features/tasks/utils/merge-tool-events.ts'

interface MinifiedEventItemProps {
  event: Event | MergedEvent
}

const MAX_TEXT_LENGTH = 100

function truncateText(text: string, maxLength: number = MAX_TEXT_LENGTH): string {
  if (text.length <= maxLength) return text
  return `${text.slice(0, maxLength)}...`
}

function getToolIcon(toolName: string) {
  const iconMap: Record<string, React.ReactNode> = {
    Read: <FileText className="h-3 w-3" />,
    Write: <FileEdit className="h-3 w-3" />,
    Edit: <Edit className="h-3 w-3" />,
    MultiEdit: <Edit className="h-3 w-3" />,
    Grep: <Search className="h-3 w-3" />,
    Glob: <FileSearch className="h-3 w-3" />,
    Bash: <Terminal className="h-3 w-3" />,
    Create: <Plus className="h-3 w-3" />,
    WebFetch: <Globe className="h-3 w-3" />,
    TodoWrite: <ListTodo className="h-3 w-3" />,
    TodoRead: <ListTodo className="h-3 w-3" />,
    LS: <FileSearch className="h-3 w-3" />,
    Task: <Bot className="h-3 w-3" />,
  }

  return iconMap[toolName] || <Code2 className="h-3 w-3" />
}

function getToolSummary(toolContent: ToolUseContent): string {
  const toolName = toolContent.name
  const input = toolContent.input as Record<string, unknown>

  if (!input || typeof input !== 'object') {
    return ''
  }

  // Provide concise summaries based on tool type
  switch (toolName) {
    case 'Read':
      return input.file_path && typeof input.file_path === 'string'
        ? truncateText(input.file_path, 40)
        : ''
    case 'Write':
      return input.file_path && typeof input.file_path === 'string'
        ? truncateText(input.file_path, 40)
        : ''
    case 'Edit':
    case 'MultiEdit':
      return input.file_path && typeof input.file_path === 'string'
        ? truncateText(input.file_path, 40)
        : ''
    case 'Grep':
      return input.pattern && typeof input.pattern === 'string'
        ? `"${truncateText(input.pattern, 30)}"`
        : ''
    case 'Glob':
      return input.pattern && typeof input.pattern === 'string'
        ? `"${truncateText(input.pattern, 30)}"`
        : ''
    case 'Bash':
      return input.command && typeof input.command === 'string'
        ? truncateText(input.command, 50)
        : ''
    case 'WebFetch':
      return input.url && typeof input.url === 'string' ? truncateText(input.url, 40) : ''
    case 'TodoWrite':
      return input.todos && Array.isArray(input.todos) ? `${input.todos.length} items` : ''
    case 'TodoRead':
      return ''
    case 'LS':
      return input.path && typeof input.path === 'string' ? truncateText(input.path, 40) : ''
    case 'Task':
      return input.description && typeof input.description === 'string'
        ? truncateText(input.description, 40)
        : ''
    default:
      return input.description && typeof input.description === 'string' ? input.description : ''
  }
}

function extractToolContent(event: Event): ToolUseContent | null {
  if (event.type !== 'message') return null

  const messageEvent = event as MessageEvent
  const payload = messageEvent.payload

  if (
    payload?.type === 'assistant' &&
    payload.message?.content &&
    Array.isArray(payload.message.content)
  ) {
    for (const content of payload.message.content) {
      if (isToolUseContent(content)) {
        return content
      }
    }
  }

  return null
}

function extractToolResult(event: Event): string | null {
  if (event.type !== 'message') return null

  const messageEvent = event as MessageEvent
  const payload = messageEvent.payload

  if (
    payload?.type === 'user' &&
    payload.message?.content &&
    Array.isArray(payload.message.content)
  ) {
    for (const content of payload.message.content) {
      if (isToolResultContent(content)) {
        return content.content || null
      }
    }
  }

  return null
}

export function MinifiedEventItem({ event }: MinifiedEventItemProps) {
  const isMerged = 'toolResult' in event

  // Check if it's a tool call
  const toolContent = extractToolContent(event)

  // For non-tool message events, check if it's a text message
  if (event.type === 'message' && !toolContent) {
    const messageEvent = event as MessageEvent
    const payload = messageEvent.payload

    // Handle text messages
    if (payload?.type === 'assistant' && payload.message?.content) {
      const content = payload.message.content

      // If content is a string, show it
      if (typeof content === 'string') {
        return (
          <div className="flex items-start gap-2 text-xs text-muted-foreground">
            <Bot className="h-3 w-3 mt-0.5 flex-shrink-0" />
            <span className="break-words">{truncateText(content)}</span>
          </div>
        )
      }

      // If content is an array with text content
      if (Array.isArray(content)) {
        const textContent = content.find((c) => c.type === 'text')
        if (textContent && 'text' in textContent) {
          return (
            <div className="flex items-start gap-2 text-xs text-muted-foreground">
              <Bot className="h-3 w-3 mt-0.5 flex-shrink-0" />
              <span className="break-words">{truncateText(textContent.text)}</span>
            </div>
          )
        }
      }
    }
  }

  // If no tool content found, return null
  if (!toolContent) {
    return null
  }

  const toolName = toolContent.name
  const summary = getToolSummary(toolContent)
  const hasError = event.isError || (isMerged && event.toolResult?.isError)

  // Extract result for merged events
  let resultSummary = ''
  if (isMerged && event.toolResult) {
    const result = extractToolResult(event.toolResult)
    if (result) {
      const lines = result.split('\n').filter((line) => line.trim())
      if (lines.length > 0) {
        resultSummary = truncateText(lines[0], 60)
      }
    }
  }

  return (
    <div className="flex items-start gap-2 text-xs">
      <div
        className={`mt-0.5 flex-shrink-0 ${hasError ? 'text-destructive' : 'text-muted-foreground'}`}
      >
        {getToolIcon(toolName)}
      </div>
      <div className="flex-1 min-w-0">
        <span className={`font-medium ${hasError ? 'text-destructive' : ''}`}>{toolName}</span>
        {summary && <span className="text-muted-foreground ml-1 break-words">{summary}</span>}
        {resultSummary && <span className="text-muted-foreground ml-1">→ {resultSummary}</span>}
      </div>
    </div>
  )
}
