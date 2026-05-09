import type { UiMessage } from '@moatless/api-client'
import { parseMessageContent } from '../message-history/assistant-message'
import { useCopyAction } from './actions/copy-action'
import { useFeedbackAction } from './actions/feedback-action'
import { MessageActionBar } from './message-action-bar'

interface LastAssistantActionsProps {
  taskId: string
  message: UiMessage
}

/**
 * Action bar rendered below the *last* assistant reply once streaming has
 * stopped. Shows `Copy` and `Feedback`. Adding new actions is a one-line
 * append in this component.
 */
export function LastAssistantActions({ taskId, message }: LastAssistantActionsProps) {
  const { textContent } = parseMessageContent(message.content)
  const copyAction = useCopyAction(textContent)
  const feedback = useFeedbackAction({
    taskId,
    surface: 'message',
    messageUuid: message.uuid,
  })
  return (
    <>
      <MessageActionBar actions={[copyAction, feedback.action]} />
      {feedback.dialog}
    </>
  )
}
