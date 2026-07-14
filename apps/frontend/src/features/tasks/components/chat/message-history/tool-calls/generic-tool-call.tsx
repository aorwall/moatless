import type { UiContentBlock } from '@moatless/api-client'
import { ChevronDown, ChevronRight, Loader2 } from 'lucide-react'
import { useTaskToolCall } from '@/features/tasks/hooks/use-tasks'
import { type ImageContentBlock, isImageContentBlock } from '@/lib/types/content-guards'
import { cn } from '@/lib/utils'
import { ImageThumbnail } from '../image-thumbnail'
import { ToolCallFeedbackChrome } from './tool-call-feedback.tsx'
import type { ToolCallProps } from './tool-call-props.ts'

/**
 * Parse tool result content to extract image data if present
 * Tool results may contain JSON with image blocks like:
 * {"type":"image","source":{"type":"base64","data":"...","media_type":"image/png"}}
 */
function parseImageFromToolResult(content: string | undefined): ImageContentBlock | null {
  if (!content) {
    return null
  }

  try {
    const parsed = JSON.parse(content)
    if (isImageContentBlock(parsed)) {
      return parsed
    }
  } catch {
    // Not JSON or invalid format, ignore
  }

  return null
}

/** Format tool input for the compact header: first parameter only. */
function formatToolInput(toolInput: UiContentBlock['toolInput']): string {
  if (!toolInput) {
    return ''
  }

  const entries = Object.entries(toolInput)
  const firstEntry = entries[0]
  if (!firstEntry) {
    return ''
  }
  const [_key, value] = firstEntry
  const displayValue = typeof value === 'string' ? value : JSON.stringify(value)
  const truncatedValue = displayValue.length > 50 ? `${displayValue.slice(0, 50)}...` : displayValue

  return entries.length > 1 ? `${truncatedValue} +${entries.length - 1} more` : truncatedValue
}

interface ExpandedToolResultProps {
  content: UiContentBlock
  detail: ReturnType<typeof useTaskToolCall>['data']
  detailIsLoading: boolean
  expandedLines: string[]
  imageData: ImageContentBlock | null
}

/** Expanded body of a generic tool call: full input, image or line output. */
function ExpandedToolResult({
  content,
  detail,
  detailIsLoading,
  expandedLines,
  imageData,
}: ExpandedToolResultProps) {
  return (
    <div className="mt-1 space-y-1">
      {detail?.toolInput && (
        <pre className="bg-muted/20 p-1 text-muted-foreground whitespace-pre-wrap break-all">
          {JSON.stringify(detail.toolInput, null, 2)}
        </pre>
      )}
      {detailIsLoading && !detail && (
        <div className="flex items-center gap-1 text-muted-foreground py-1">
          <Loader2 className="h-3 w-3 animate-spin" />
          Loading…
        </div>
      )}
      {imageData ? (
        <div className="p-1">
          <ImageThumbnail
            data={imageData.source.data}
            mediaType={imageData.source.media_type}
            alt={`${content.toolName} result`}
            size="expanded"
          />
        </div>
      ) : (
        expandedLines.length > 0 && (
          <div className="bg-muted/30 p-1 text-muted-foreground">
            {expandedLines.map((line, index) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: output lines render in a fixed order
              <div key={`${content.toolUseId}-${index}`} className="whitespace-pre-wrap break-all">
                {line || ' '}
              </div>
            ))}
          </div>
        )
      )}
    </div>
  )
}

/** Fallback renderer for tool calls without a dedicated renderer. */
export function GenericToolCall({
  taskId,
  parentMessageUuid,
  content,
  toolResult,
  expanded,
  onToggle,
}: ToolCallProps) {
  // Fetch full tool input + result on demand when this tool call is expanded.
  // The list endpoint returns a slim preview to keep page/SSE refresh payloads
  // small; the detail query stays cached across collapse/re-expand.
  const detailQuery = useTaskToolCall(taskId ?? '', content.toolUseId, expanded && !!taskId)
  const detail = detailQuery.data
  const detailIsLoading = detailQuery.isLoading

  const hasResult = !!toolResult
  // Prefer the full content from the on-demand detail query when available,
  // falling back to the slim preview shipped on the list. This keeps the line
  // count + scroll height stable as the full data hydrates.
  const expandedContent = detail?.resultContent ?? toolResult?.content ?? undefined
  const imageData = parseImageFromToolResult(expandedContent)
  const resultLines = (toolResult?.content ?? '').split('\n')
  const lineCount = toolResult?.content ? resultLines.length : 0
  const expandedLines = expandedContent ? expandedContent.split('\n') : resultLines
  const isExpandable = hasResult && lineCount > 0

  return (
    <div className="text-xs font-mono w-full max-w-full group">
      <div className="flex items-start gap-2">
        <div
          className={cn(
            'w-1.5 h-1.5 rounded-full mt-1 flex-shrink-0',
            toolResult?.isError ? 'bg-destructive' : 'bg-primary',
          )}
        />
        <div className="flex-1 min-w-0">
          {/* Clickable header */}
          {/* biome-ignore lint/a11y/noStaticElementInteractions: Role and keyboard handler are conditionally added when interactive */}
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
              'flex items-center gap-1 min-w-0',
              isExpandable && 'cursor-pointer hover:text-foreground transition-colors',
            )}
          >
            {isExpandable && (
              <span className="flex-shrink-0">
                {expanded ? (
                  <ChevronDown className="h-3 w-3" />
                ) : (
                  <ChevronRight className="h-3 w-3" />
                )}
              </span>
            )}
            <span className="font-medium flex-shrink-0">{content.toolName}</span>
            {content.toolInput && (
              <span className="text-muted-foreground truncate min-w-0">
                ({formatToolInput(content.toolInput)})
              </span>
            )}
            {isExpandable && (
              <span className="text-muted-foreground flex-shrink-0">
                {lineCount} {lineCount === 1 ? 'line' : 'lines'}
              </span>
            )}
            <ToolCallFeedbackChrome
              taskId={taskId}
              parentMessageUuid={parentMessageUuid}
              toolUseId={content.toolUseId ?? undefined}
            />
          </div>

          {/* Result - only shown when expanded */}
          {hasResult && expanded && (
            <ExpandedToolResult
              content={content}
              detail={detail}
              detailIsLoading={detailIsLoading}
              expandedLines={expandedLines}
              imageData={imageData}
            />
          )}
        </div>
      </div>
    </div>
  )
}
