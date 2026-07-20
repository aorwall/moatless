import type { UiContentBlock, UiMessage, UiToolResult } from '@moatless/api-client'
import { ChevronDown, ChevronRight, Eye, FileText } from 'lucide-react'
import { Response } from '@/lib/components/ai-elements/response'
import { cn } from '@/lib/utils'
import { ExternalSourceChip } from './external-source-chip.tsx'
import { renderToolCalls } from './tool-calls/tool-call-list.tsx'

/**
 * Render a collapsed message view (for hidden/transcript-only messages)
 */
export function renderCollapsedMessage(
  taskId: string | undefined,
  isExpanded: boolean,
  setIsExpanded: (value: boolean) => void,
  isSummary: boolean,
  previewText: string,
  textContent: string,
  toolCalls: UiContentBlock[],
  toolResultMap: Map<string, UiToolResult>,
  expandedToolCalls: Set<string>,
  onToggleToolCall: (toolUseId: string) => void,
  hasPendingUserInput: boolean | null | undefined,
  externalSource: UiMessage['externalSource'] | null | undefined,
  subagentsByToolUseId?: Record<string, UiMessage[]>,
  parentMessageUuid?: string,
): React.ReactNode {
  return (
    <div
      className={cn(
        'border border-dashed rounded-lg overflow-hidden',
        isSummary
          ? 'border-purple-500/30 bg-purple-500/5'
          : 'border-muted-foreground/30 bg-muted/20',
      )}
    >
      {externalSource && <ExternalSourceChip source={externalSource} />}
      <button
        type="button"
        onClick={() => setIsExpanded(!isExpanded)}
        className="w-full flex items-center gap-2 p-2 text-xs text-muted-foreground hover:bg-muted/30 transition-colors"
      >
        {isExpanded ? (
          <ChevronDown className="h-3 w-3 flex-shrink-0" />
        ) : (
          <ChevronRight className="h-3 w-3 flex-shrink-0" />
        )}
        {isSummary ? (
          <>
            <FileText className="h-3 w-3 flex-shrink-0 text-purple-500" />
            <span className="font-medium">Summary Response</span>
          </>
        ) : (
          <>
            <Eye className="h-3 w-3 flex-shrink-0" />
            <span className="font-medium">Hidden Response</span>
          </>
        )}
        <span className="text-muted-foreground/70 truncate">
          {previewText.slice(0, 50)}
          {previewText.length > 50 ? '...' : ''}
        </span>
      </button>

      {isExpanded && (
        <div className="p-3 border-t border-dashed border-muted-foreground/20">
          <div className="space-y-2 opacity-80">
            {textContent && (
              <div className="text-sm w-full max-w-full">
                <Response>{textContent}</Response>
              </div>
            )}

            {renderToolCalls(
              taskId,
              toolCalls,
              toolResultMap,
              expandedToolCalls,
              onToggleToolCall,
              hasPendingUserInput,
              subagentsByToolUseId,
              parentMessageUuid,
            )}
          </div>
        </div>
      )}
    </div>
  )
}
