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
import type React from 'react'
import { isToolUseContent, type ToolUseContent } from '@/lib/types/content-guards'
import type { AgentEvent, Event } from '@/lib/types/events'

interface MinifiedEventItemProps {
  event: Event
}

const MAX_TEXT_LENGTH = 100

function truncateText(text: string, maxLength: number = MAX_TEXT_LENGTH): string {
  if (text.length <= maxLength) {
    return text
  }
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
    'codex.shell': <Terminal className="h-3 w-3" />,
    'codex.edit': <FileEdit className="h-3 w-3" />,
    'codex.todo_list': <ListTodo className="h-3 w-3" />,
  }

  return iconMap[toolName] || <Code2 className="h-3 w-3" />
}

// Helper: Get truncated file path from input
function getFilePathSummary(input: Record<string, unknown>, maxLength = 40): string {
  return input.file_path && typeof input.file_path === 'string'
    ? truncateText(input.file_path, maxLength)
    : ''
}

// Helper: Get truncated pattern from input
function getPatternSummary(input: Record<string, unknown>, maxLength = 30): string {
  return input.pattern && typeof input.pattern === 'string'
    ? `"${truncateText(input.pattern, maxLength)}"`
    : ''
}

// Helper: Get truncated command from input
function getCommandSummary(input: Record<string, unknown>, maxLength = 50): string {
  return input.command && typeof input.command === 'string'
    ? truncateText(input.command, maxLength)
    : ''
}

// Helper: Get truncated URL from input
function getUrlSummary(input: Record<string, unknown>, maxLength = 40): string {
  return input.url && typeof input.url === 'string' ? truncateText(input.url, maxLength) : ''
}

// Helper: Get truncated path from input
function getPathSummary(input: Record<string, unknown>, maxLength = 40): string {
  return input.path && typeof input.path === 'string' ? truncateText(input.path, maxLength) : ''
}

// Helper: Get truncated description from input
function getDescriptionSummary(input: Record<string, unknown>, maxLength = 40): string {
  return input.description && typeof input.description === 'string'
    ? truncateText(input.description, maxLength)
    : ''
}

// Helper: Get todo count summary
function getTodoCountSummary(input: Record<string, unknown>): string {
  return input.todos && Array.isArray(input.todos) ? `${input.todos.length} items` : ''
}

// Helper: Get codex todo_list item count summary
function getCodexTodoListSummary(input: Record<string, unknown>): string {
  return Array.isArray(input.items) ? `${input.items.length} items` : ''
}

// Helper: Get codex edit change summary
function getCodexEditSummary(input: Record<string, unknown>): string {
  if (!Array.isArray(input.changes) || input.changes.length === 0) {
    return ''
  }
  if (input.changes.length === 1) {
    const first = input.changes[0] as { path?: unknown }
    return typeof first.path === 'string' ? truncateText(first.path, 40) : ''
  }
  return `${input.changes.length} files`
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
    case 'Write':
    case 'Edit':
    case 'MultiEdit':
      return getFilePathSummary(input)
    case 'Grep':
    case 'Glob':
      return getPatternSummary(input)
    case 'Bash':
      return getCommandSummary(input)
    case 'WebFetch':
      return getUrlSummary(input)
    case 'TodoWrite':
      return getTodoCountSummary(input)
    case 'TodoRead':
      return ''
    case 'LS':
      return getPathSummary(input)
    case 'Task':
      return getDescriptionSummary(input)
    case 'codex.shell':
      return getCommandSummary(input)
    case 'codex.edit':
      return getCodexEditSummary(input)
    case 'codex.todo_list':
      return getCodexTodoListSummary(input)
    default:
      return getDescriptionSummary(input)
  }
}

function extractToolContent(event: Event): ToolUseContent | null {
  if (event.type !== 'agent' || event.subType !== 'message') {
    return null
  }

  const agentEvent = event as AgentEvent
  const payload = agentEvent.payload

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

export function MinifiedEventItem({ event }: MinifiedEventItemProps) {
  // Check if it's a tool call
  const toolContent = extractToolContent(event)

  // For non-tool message events, check if it's a text message
  if (event.type === 'agent' && event.subType === 'message' && !toolContent) {
    const agentEvent = event as AgentEvent
    const payload = agentEvent.payload

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
  const hasError = event.isError

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
      </div>
    </div>
  )
}
