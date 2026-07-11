import { Sparkles } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { ImperativePanelHandle } from 'react-resizable-panels'
import { Button } from '@/lib/components/ui/button'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { cn } from '@/lib/utils'
import { TaskKioskHeader } from '../components/task-header/task-kiosk-header'
import { usePanelControls } from '../contexts/task-providers'
import { ChatPanel } from '../panels/chat-panel'
import { WorkspacePanel } from '../panels/workspace-panel'

/**
 * Kiosk task layout: the chrome-less on-display variant. Workspace sits on the
 * left, chat (collapsible) on the right. Sets the kiosk flags at the
 * composition site so the panels stay layout-agnostic.
 */
export function KioskTaskLayout() {
  const { registerChatToggle } = usePanelControls()
  const chatPanelRef = useRef<ImperativePanelHandle>(null)
  const [isChatCollapsed, setIsChatCollapsed] = useState(false)

  const handleExpandChat = () => {
    chatPanelRef.current?.expand()
  }

  const toggleChat = useCallback(() => {
    const panel = chatPanelRef.current
    if (!panel) {
      return
    }
    if (panel.isCollapsed()) {
      panel.expand()
    } else {
      panel.collapse()
    }
  }, [])

  useEffect(() => registerChatToggle(toggleChat), [registerChatToggle, toggleChat])

  return (
    <div className="h-full w-full relative">
      <ResizablePanelGroup direction="horizontal" className="h-full w-full">
        <ResizablePanel className="flex flex-col overflow-hidden">
          <WorkspacePanel kiosk />
        </ResizablePanel>

        <ResizableHandle
          withHandle
          className={cn(
            isChatCollapsed && 'bg-primary/20 w-1 hover:bg-primary/40 transition-colors',
          )}
        />

        <ResizablePanel
          ref={chatPanelRef}
          defaultSize={40}
          minSize={10}
          maxSize={60}
          collapsible={true}
          collapsedSize={0}
          onCollapse={() => setIsChatCollapsed(true)}
          onExpand={() => setIsChatCollapsed(false)}
          className="flex flex-col overflow-hidden"
        >
          <div className="flex-1 min-h-0 flex flex-col relative">
            <TaskKioskHeader />
            <ChatPanel kiosk />
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>

      {/* Expand button for collapsed chat panel */}
      {isChatCollapsed && (
        <Button
          onClick={handleExpandChat}
          className="absolute right-2 top-2 z-10"
          size="icon"
          variant="outline"
          aria-label="Expand chat panel"
        >
          <Sparkles className="h-4 w-4" />
        </Button>
      )}
    </div>
  )
}
