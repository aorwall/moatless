import { useCallback, useEffect } from 'react'
import { TaskMobileHeader } from '../components/task-header/task-mobile-header'
import { usePanelControls } from '../contexts/task-providers'
import { useTaskView } from '../hooks/use-task-view'
import { ChatPanel } from '../panels/chat-panel'
import { WorkspacePanel } from '../panels/workspace-panel'

/**
 * Mobile task layout: one panel at a time (chat or workspace), selected by the
 * `?view=` param. `toggleChat` is mapped to switching back to the chat view.
 */
export function MobileTaskLayout() {
  const { registerChatToggle } = usePanelControls()
  const { view, setView } = useTaskView({ fallback: 'chat' })

  const showChat = useCallback(() => setView('chat'), [setView])
  useEffect(() => registerChatToggle(showChat), [registerChatToggle, showChat])

  return (
    <div className="h-full w-full flex flex-col overflow-hidden">
      <div className="flex-1 min-h-0 relative">
        <TaskMobileHeader />
        {view === 'chat' ? <ChatPanel /> : <WorkspacePanel />}
      </div>
    </div>
  )
}
