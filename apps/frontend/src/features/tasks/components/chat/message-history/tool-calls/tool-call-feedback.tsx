import { MessageSquare, MessageSquarePlus } from 'lucide-react'
import { useState } from 'react'
import { FeedbackDialog } from '@/features/feedback/components/feedback-dialog'
import { useTaskFeedbackIndex } from '@/features/feedback/hooks/use-task-feedback'

/**
 * Hover-revealed feedback button + always-visible "has feedback" indicator
 * shown next to a tool call header. Returns `null` when no taskId is known
 * (sandbox preview mode etc.) so the chrome stays uncluttered.
 */
export function ToolCallFeedbackChrome({
  taskId,
  parentMessageUuid,
  toolUseId,
}: {
  taskId?: string | undefined
  parentMessageUuid?: string | undefined
  toolUseId?: string | undefined
}) {
  const [open, setOpen] = useState(false)
  const index = useTaskFeedbackIndex(taskId)
  if (!(taskId && toolUseId)) {
    return null
  }
  const count = index.byToolUseId.get(toolUseId) ?? 0
  return (
    <span className="ml-auto flex items-center gap-1 flex-shrink-0">
      {count > 0 && (
        <span
          className="inline-flex items-center gap-0.5 text-[10px] text-muted-foreground"
          title={`${count} feedback note${count === 1 ? '' : 's'}`}
        >
          <MessageSquare className="h-3 w-3 fill-current" />
          {count}
        </span>
      )}
      <button
        type="button"
        aria-label="Give feedback on this tool call"
        title="Give feedback on this tool call"
        onClick={(e) => {
          e.stopPropagation()
          setOpen(true)
        }}
        className="opacity-0 group-hover:opacity-100 focus:opacity-100 rounded p-0.5 hover:bg-muted hover:text-foreground transition-opacity"
      >
        <MessageSquarePlus className="h-3 w-3" />
      </button>
      <FeedbackDialog
        open={open}
        onOpenChange={setOpen}
        taskId={taskId}
        messageUuid={parentMessageUuid}
        toolUseId={toolUseId}
        surface="tool_call"
      />
    </span>
  )
}
