import type { UiContentBlock, UiToolResult } from '@moatless/api-client'
import {
  CheckCircle,
  ChevronDown,
  ChevronRight,
  FileEdit,
  ListTodo,
  Terminal,
  XCircle,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import { type Todo, TodoList } from './todo-list.tsx'

interface CodexToolCallProps {
  content: UiContentBlock
  toolResult?: UiToolResult
  expanded: boolean
  onToggle: () => void
}

interface CodexShellInput {
  command?: string
  cwd?: string | null
}

interface CodexShellResult {
  status?: 'in_progress' | 'completed' | 'failed' | 'declined'
  exitCode?: number | null
  aggregatedOutput?: string | null
  durationMs?: number | null
}

interface CodexEditChange {
  path: string
  kind: 'add' | 'update' | 'delete'
  diff?: string | null
}

interface CodexEditInput {
  changes?: CodexEditChange[]
}

interface CodexEditResult {
  status?: 'in_progress' | 'completed' | 'failed' | 'declined'
  changes?: CodexEditChange[]
}

interface CodexTodoItem {
  text: string
  completed: boolean
}

interface CodexTodoListInput {
  items?: CodexTodoItem[]
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}...` : text
}

function ResultIcon({ isError }: { isError: boolean }) {
  return isError ? (
    <XCircle className="h-3 w-3 text-red-500 flex-shrink-0" />
  ) : (
    <CheckCircle className="h-3 w-3 text-green-500 flex-shrink-0" />
  )
}

interface ExpandableHeaderProps {
  isExpandable: boolean
  expanded: boolean
  onToggle: () => void
  children: React.ReactNode
}

function ExpandableHeader({ isExpandable, expanded, onToggle, children }: ExpandableHeaderProps) {
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: expandable header
    <div
      role={isExpandable ? 'button' : undefined}
      tabIndex={isExpandable ? 0 : undefined}
      onClick={isExpandable ? onToggle : undefined}
      onKeyDown={
        isExpandable
          ? (e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault()
                onToggle()
              }
            }
          : undefined
      }
      className={cn(
        'flex items-center gap-1.5 min-w-0',
        isExpandable && 'cursor-pointer hover:text-foreground transition-colors',
      )}
    >
      {isExpandable && (
        <span className="flex-shrink-0">
          {expanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        </span>
      )}
      {children}
    </div>
  )
}

function ExitCodePill({ exitCode }: { exitCode: number }) {
  return (
    <span
      className={cn(
        'px-1.5 rounded text-[10px]',
        exitCode === 0
          ? 'bg-green-500/10 text-green-600 dark:text-green-400'
          : 'bg-red-500/10 text-red-600 dark:text-red-400',
      )}
    >
      exit {exitCode}
    </span>
  )
}

function isShellError(
  toolResult: UiToolResult | undefined,
  result: CodexShellResult | null,
  exitCode: number | null,
): boolean {
  if (toolResult?.isError === true) {
    return true
  }
  if (result?.status === 'failed' || result?.status === 'declined') {
    return true
  }
  return typeof exitCode === 'number' && exitCode !== 0
}

function CodexShellCall({ content, toolResult, expanded, onToggle }: CodexToolCallProps) {
  const input = (content.toolInput ?? {}) as CodexShellInput
  const result = (toolResult?.typedResult ?? null) as CodexShellResult | null
  const command = input.command ?? ''
  const exitCode = result?.exitCode ?? null
  const output = result?.aggregatedOutput ?? ''
  const hasResult = !!toolResult
  const isError = isShellError(toolResult, result, exitCode)
  const outputLines = output ? output.split('\n') : []
  const hasOutput = outputLines.some((l) => l.length > 0)
  const isExpandable = hasResult && hasOutput

  return (
    <div className="text-xs font-mono w-full max-w-full">
      <div className="flex items-start gap-2">
        <div
          className={cn(
            'w-1.5 h-1.5 rounded-full mt-1 flex-shrink-0',
            isError ? 'bg-destructive' : 'bg-primary',
          )}
        />
        <div className="flex-1 min-w-0">
          <ExpandableHeader isExpandable={isExpandable} expanded={expanded} onToggle={onToggle}>
            <Terminal className="h-3 w-3 text-primary flex-shrink-0" />
            <span className="font-medium flex-shrink-0">Shell</span>
            <span className="text-muted-foreground truncate min-w-0">{truncate(command, 80)}</span>
            {input.cwd && (
              <span className="text-muted-foreground flex-shrink-0">
                ({truncate(input.cwd, 30)})
              </span>
            )}
            {hasResult && (
              <span className="flex items-center gap-1 flex-shrink-0 ml-auto">
                {typeof exitCode === 'number' && <ExitCodePill exitCode={exitCode} />}
                <ResultIcon isError={isError} />
              </span>
            )}
          </ExpandableHeader>

          {isExpandable && expanded && (
            <div className="mt-1 bg-muted/30 p-1 text-muted-foreground max-h-96 overflow-y-auto">
              {outputLines.map((line, index) => (
                <div
                  // biome-ignore lint/suspicious/noArrayIndexKey: output lines have no stable id
                  key={`${content.toolUseId}-line-${index}`}
                  className="whitespace-pre-wrap break-all"
                >
                  {line || ' '}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

function changeBadgeClass(kind: CodexEditChange['kind']) {
  switch (kind) {
    case 'add':
      return 'bg-green-500/10 text-green-600 dark:text-green-400'
    case 'update':
      return 'bg-blue-500/10 text-blue-600 dark:text-blue-400'
    case 'delete':
      return 'bg-red-500/10 text-red-600 dark:text-red-400'
  }
}

function CodexEditCall({ content, toolResult, expanded, onToggle }: CodexToolCallProps) {
  const input = (content.toolInput ?? {}) as CodexEditInput
  const result = (toolResult?.typedResult ?? null) as CodexEditResult | null
  const changes = input.changes ?? result?.changes ?? []
  const hasResult = !!toolResult
  const isError =
    toolResult?.isError === true || result?.status === 'failed' || result?.status === 'declined'
  const isExpandable = changes.length > 0

  return (
    <div className="text-xs font-mono w-full max-w-full">
      <div className="flex items-start gap-2">
        <div
          className={cn(
            'w-1.5 h-1.5 rounded-full mt-1 flex-shrink-0',
            isError ? 'bg-destructive' : 'bg-primary',
          )}
        />
        <div className="flex-1 min-w-0">
          <ExpandableHeader isExpandable={isExpandable} expanded={expanded} onToggle={onToggle}>
            <FileEdit className="h-3 w-3 text-primary flex-shrink-0" />
            <span className="font-medium flex-shrink-0">Edit</span>
            <span className="text-muted-foreground truncate min-w-0">
              {changes.length === 1 ? truncate(changes[0].path, 60) : `${changes.length} files`}
            </span>
            {hasResult && <ResultIcon isError={isError} />}
          </ExpandableHeader>

          {isExpandable && expanded && (
            <div className="mt-2 bg-muted/30 p-2 rounded space-y-1">
              {changes.map((change) => (
                <div
                  key={`${content.toolUseId}-${change.path}-${change.kind}`}
                  className="flex items-center gap-2"
                >
                  <span
                    className={cn(
                      'px-1.5 rounded text-[10px] font-medium uppercase flex-shrink-0',
                      changeBadgeClass(change.kind),
                    )}
                  >
                    {change.kind}
                  </span>
                  <span className="break-all">{change.path}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

function CodexTodoListCall({ content }: CodexToolCallProps) {
  const input = (content.toolInput ?? {}) as CodexTodoListInput
  const rawItems = input.items ?? []
  const todos: Todo[] = rawItems.map((item, index) => ({
    id: String(index),
    content: item.text,
    status: item.completed ? 'completed' : 'pending',
  }))
  const completed = todos.filter((t) => t.status === 'completed').length
  const total = todos.length

  return (
    <div className="text-xs font-mono w-full max-w-full">
      <div className="flex items-start gap-2">
        <div className="w-1.5 h-1.5 bg-primary rounded-full mt-1 flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5 mb-2 min-w-0">
            <ListTodo className="h-3 w-3 text-primary flex-shrink-0" />
            <span className="font-medium flex-shrink-0">Plan</span>
            <span className="text-muted-foreground flex-shrink-0">
              ({completed}/{total} done)
            </span>
          </div>
          <TodoList todos={todos} />
        </div>
      </div>
    </div>
  )
}

/**
 * Render a codex-specific tool call. Dispatches by tool name to a
 * terminal/shell view, edit view, todo-list plan view, or a fallback.
 *
 * Returns `null` for unknown codex.* tool names so the caller can fall
 * through to the generic renderer.
 */
export function CodexToolCall(props: CodexToolCallProps): React.ReactNode {
  const toolName = props.content.toolName
  if (toolName === 'codex.shell') {
    return <CodexShellCall {...props} />
  }
  if (toolName === 'codex.edit') {
    return <CodexEditCall {...props} />
  }
  if (toolName === 'codex.todo_list') {
    return <CodexTodoListCall {...props} />
  }
  return null
}
