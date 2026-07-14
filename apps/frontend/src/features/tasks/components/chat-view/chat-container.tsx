import type { MessageContextInput, UiMessage } from '@moatless/api-client'
import { useMemo } from 'react'
import { ChatScrollArea } from '@/features/tasks/components/chat/chat-scroll-area'
import { MessageHistory } from '@/features/tasks/components/chat/message-history'
import { usePanelControls, useTaskId } from '@/features/tasks/contexts/task-providers'
import { useSandboxRetry, useSandboxStatus } from '@/features/tasks/hooks/use-sandbox-status'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useTaskMessages } from '@/features/tasks/hooks/use-tasks'
import { ChatInputSection } from './chat-input-section'
import { SandboxErrorBanner } from './sandbox-error-banner'
import { usePendingUserInput } from './use-pending-user-input'

interface ChatContainerProps {
  className?: string
  contextItems?: MessageContextInput[]
  onContextItemsChange?: (contexts: MessageContextInput[]) => void
  /** Hide the agent edit/plan mode toggle in the chat input. */
  hideAgentModeSelector?: boolean
  /**
   * Wrap each assistant message's tool calls in a single collapsed group
   * (always shown, even for one call) that displays a live "N commands"
   * counter. Defaults to false. Used by kiosk mode.
   */
  collapseToolCalls?: boolean
}

/**
 * Composes the chat panel: message history on top, input section below.
 *
 * Owns the message list (and state derived from it, like pending user input)
 * so the history only re-renders when messages change. All per-keystroke
 * state lives in ChatInputSection, below the history in the tree — typing
 * must never re-render the transcript.
 */
function ChatContainer({
  className,
  contextItems = [],
  onContextItemsChange,
  hideAgentModeSelector = false,
  collapseToolCalls = false,
}: ChatContainerProps) {
  const taskId = useTaskId()
  const { navigatePreview } = usePanelControls()

  const { data: task } = useTaskDetailQuery(taskId)

  // Poll sandbox status only while the task remains open (hook handles gating)
  const sandboxStatusQuery = useSandboxStatus(taskId, task)
  const sandboxStatus = sandboxStatusQuery.data
  const envStatus = sandboxStatus?.environmentStatus

  // Retry sandbox mutation for handling failed provisioning
  const retrySandboxMutation = useSandboxRetry(taskId)

  // Fetch messages for this task (infinite, cursor-paginated).
  const {
    data: messagesData,
    isLoading: isLoadingMessages,
    error: messagesError,
    hasPreviousPage,
    fetchPreviousPage,
    isFetchingPreviousPage,
  } = useTaskMessages(taskId, { enabled: !!taskId })

  // Flatten loaded pages into a single chronological list.
  // TanStack Query v5 prepends pages fetched via `fetchPreviousPage`
  // (`addToStart`), so `pages[0]` is the OLDEST cached page and `pages[last]`
  // is the newest. Each page is itself oldest-first. Plain flatMap therefore
  // yields a chronological array — no reverse.
  const messages = useMemo(() => {
    if (!messagesData) {
      return
    }
    return messagesData.pages.flatMap((p) => p.messages)
  }, [messagesData])

  // Agent running state: driven by sandbox status (updated via SSE events + backend gRPC)
  const isTaskRunning = sandboxStatus?.agentState === 'running'

  // Derive queued messages from the messages list
  const queuedMessages = useMemo(
    () => (messages ?? []).filter((m): m is UiMessage => m.role === 'user' && m.isQueued === true),
    [messages],
  )

  // Detect pending user input from the last message
  const pendingUserInput = usePendingUserInput(messages)

  return (
    <ChatScrollArea className={className}>
      <MessageHistory
        taskId={taskId}
        messages={messages}
        isLoading={isLoadingMessages}
        error={messagesError as Error | null}
        isRunning={isTaskRunning}
        environmentStatus={envStatus}
        environmentError={sandboxStatus?.environmentError ?? undefined}
        onPreviewUrlClick={navigatePreview}
        hasPreviousPage={hasPreviousPage}
        isFetchingPreviousPage={isFetchingPreviousPage}
        fetchPreviousPage={fetchPreviousPage}
        collapseToolCalls={collapseToolCalls}
      />

      {/* Sandbox error banner with retry button */}
      {envStatus === 'error' && sandboxStatus?.environmentError && (
        <SandboxErrorBanner
          errorMessage={sandboxStatus.environmentError}
          onRetry={() => retrySandboxMutation.mutate()}
          isRetrying={retrySandboxMutation.isPending}
        />
      )}

      <ChatInputSection
        contextItems={contextItems}
        onContextItemsChange={onContextItemsChange}
        hideAgentModeSelector={hideAgentModeSelector}
        pendingUserInput={pendingUserInput}
        queuedMessages={queuedMessages}
      />
    </ChatScrollArea>
  )
}

export { ChatContainer }
