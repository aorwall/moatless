import { MessageSquarePlus } from 'lucide-react'
import { useState } from 'react'
import type { FeedbackSurface } from '@/features/feedback/api/feedback.api'
import { FeedbackDialog } from '@/features/feedback/components/feedback-dialog'
import type { MessageAction } from '../message-action-bar'

interface FeedbackActionConfig {
  taskId: string
  surface: FeedbackSurface
  messageUuid?: string
  toolUseId?: string
  metadata?: Record<string, unknown>
  count?: number
  label?: string
}

/**
 * Hook returning a `[MessageAction, dialog]` tuple. Render the dialog
 * somewhere stable and append the action to the bar.
 */
export function useFeedbackAction(config: FeedbackActionConfig): {
  action: MessageAction
  dialog: React.ReactElement
} {
  const [open, setOpen] = useState(false)
  const action: MessageAction = {
    key: 'feedback',
    icon: MessageSquarePlus,
    label: config.label ?? 'Give feedback',
    onClick: () => setOpen(true),
    badge:
      config.count && config.count > 0 ? (
        <span className="text-[10px] text-muted-foreground">{config.count}</span>
      ) : undefined,
  }
  const dialog = (
    <FeedbackDialog
      open={open}
      onOpenChange={setOpen}
      taskId={config.taskId}
      messageUuid={config.messageUuid}
      toolUseId={config.toolUseId}
      surface={config.surface}
      metadata={config.metadata}
    />
  )
  return { action, dialog }
}
