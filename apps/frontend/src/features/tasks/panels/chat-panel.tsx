import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ChatContainer } from '../components/chat-view/chat-container'
import { useComposer } from '../contexts/task-providers'

interface ChatPanelProps {
  className?: string
  /** Kiosk variant: hide the agent-mode selector and collapse tool calls. */
  kiosk?: boolean
}

/**
 * The chat side of the task page: the message history + composer, wired to the
 * task composer context and wrapped in an error boundary. Layouts arrange it;
 * this panel owns no layout of its own.
 */
export function ChatPanel({ className = 'flex-1 min-h-0', kiosk = false }: ChatPanelProps) {
  const composer = useComposer()

  return (
    <ErrorBoundary componentName="Chat View">
      <ChatContainer
        className={className}
        contextItems={composer.contextItems}
        onContextItemsChange={composer.setContextItems}
        hideAgentModeSelector={kiosk}
        collapseToolCalls={kiosk}
      />
    </ErrorBoundary>
  )
}
