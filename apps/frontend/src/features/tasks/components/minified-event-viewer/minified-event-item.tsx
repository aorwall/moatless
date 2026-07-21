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
  const filePath = input['file_path']
  return filePath && typeof filePath === 'string' ? truncateText(filePath, maxLength) : ''
}

// Helper: Get truncated pattern from input
function getPatternSummary(input: Record<string, unknown>, maxLength = 30): string {
  const pattern = input['pattern']
  return pattern && typeof pattern === 'string' ? `"${truncateText(pattern, maxLength)}"` : ''
}

// Helper: Get truncated command from input
function getCommandSummary(input: Record<string, unknown>, maxLength = 50): string {
  const command = input['command']
  return command && typeof command === 'string' ? truncateText(command, maxLength) : ''
}

// Helper: Get truncated URL from input
function getUrlSummary(input: Record<string, unknown>, maxLength = 40): string {
  const url = input['url']
  return url && typeof url === 'string' ? truncateText(url, maxLength) : ''
}

// Helper: Get truncated path from input
function getPathSummary(input: Record<string, unknown>, maxLength = 40): string {
  const path = input['path']
  return path && typeof path === 'string' ? truncateText(path, maxLength) : ''
}

// Helper: Get truncated description from input
function getDescriptionSummary(input: Record<string, unknown>, maxLength = 40): string {
  const description = input['description']
  return description && typeof description === 'string' ? truncateText(description, maxLength) : ''
}

// Helper: Get todo count summary
function getTodoCountSummary(input: Record<string, unknown>): string {
  const todos = input['todos']
  return todos && Array.isArray(todos) ? `${todos.length} items` : ''
}

// Helper: Get codex todo_list item count summary
function getCodexTodoListSummary(input: Record<string, unknown>): string {
  const items = input['items']
  return Array.isArray(items) ? `${items.length} items` : ''
}

// Helper: Get codex edit change summary
function getCodexEditSummary(input: Record<string, unknown>): string {
  const changes = input['changes']
  if (!Array.isArray(changes) || changes.length === 0) {
    return ''
  }
  if (changes.length === 1) {
    const first = changes[0] as { path?: unknown }
    return typeof first.path === 'string' ? truncateText(first.path, 40) : ''
  }
  return `${changes.length} files`
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
    payload.message?.['content'] &&
    Array.isArray(payload.message['content'])
  ) {
    for (const content of payload.message['content']) {
      if (isToolUseContent(content)) {
        return content
      }
    }
  }

  return null
}

/** Text of the first text block in a raw SDK content value, if any. */
function agentMessageText(payload: AgentEvent['payload']): string | null {
  if (payload?.type !== 'assistant' || !payload.message?.['content']) {
    return null
  }
  const content = payload.message['content']
  if (typeof content === 'string') {
    return content
  }
  if (Array.isArray(content)) {
    const textContent = content.find((c) => c.type === 'text')
    if (textContent && 'text' in textContent) {
      return textContent.text
    }
  }
  return null
}

/**
 * Render a non-tool `agent.message`: its assistant text when the source event
 * still carries content, else the skeleton's `toolNames` (ADR-0003) so the
 * debug timeline still shows tool activity. Returns null for anything else.
 */
function NonToolAgentMessage({ event }: { event: Event }): React.ReactNode {
  if (event.type !== 'agent' || event.subType !== 'message') {
    return null
  }

  const text = agentMessageText((event as AgentEvent).payload)
  if (text) {
    return (
      <div className="flex items-start gap-2 text-xs text-muted-foreground">
        <Bot className="h-3 w-3 mt-0.5 flex-shrink-0" />
        <span className="break-words">{truncateText(text)}</span>
      </div>
    )
  }

  const toolNames = (event.payload as { toolNames?: unknown }).toolNames
  if (Array.isArray(toolNames) && toolNames.length > 0) {
    const names = toolNames.map(String)
    return (
      <div className="flex items-start gap-2 text-xs">
        <div className="mt-0.5 flex-shrink-0 text-muted-foreground">
          {getToolIcon(names[0] ?? '')}
        </div>
        <div className="flex-1 min-w-0">
          <span className="font-medium">{names.join(', ')}</span>
        </div>
      </div>
    )
  }
  return null
}

export function MinifiedEventItem({ event }: MinifiedEventItemProps) {
  // Check if it's a tool call
  const toolContent = extractToolContent(event)

  // Non-tool agent messages (text, or the skeleton's tool names) render via the
  // dedicated helper.
  if (!toolContent) {
    return <NonToolAgentMessage event={event} />
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
