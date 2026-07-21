import { ChevronRight } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { ImperativePanelHandle } from 'react-resizable-panels'
import { Button } from '@/lib/components/ui/button'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { cn } from '@/lib/utils'
import { TaskLeftHeader } from '../components/task-header/task-left-header'
import { usePanelControls } from '../contexts/task-providers'
import { ChatPanel } from '../panels/chat-panel'
import { WorkspacePanel } from '../panels/workspace-panel'

/**
 * Desktop task layout: a resizable chat│workspace split. Chat sits on the left
 * (collapsible), the workspace on the right. Owns the chat-collapse behavior and
 * registers it as the `toggleChat` panel control.
 */
export function DesktopTaskLayout() {
  const { registerChatToggle } = usePanelControls()
  const chatPanelRef = useRef<ImperativePanelHandle>(null)
  const [isChatCollapsed, setIsChatCollapsed] = useState(false)

  const handleExpandChat = () => {
    chatPanelRef.current?.expand()
  }

  useEffect(
    () =>
      registerChatToggle(() => {
        const panel = chatPanelRef.current
        if (!panel) {
          return
        }
        if (panel.isCollapsed()) {
          panel.expand()
        } else {
          panel.collapse()
        }
      }),
    [registerChatToggle],
  )

  return (
    <div className="h-full w-full relative">
      <ResizablePanelGroup direction="horizontal" className="h-full w-full">
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
            <TaskLeftHeader />
            <ChatPanel />
          </div>
        </ResizablePanel>

        <ResizableHandle
          withHandle
          className={cn(
            isChatCollapsed && 'bg-primary/20 w-1 hover:bg-primary/40 transition-colors',
          )}
        />

        <ResizablePanel className="flex flex-col overflow-hidden">
          <WorkspacePanel />
        </ResizablePanel>
      </ResizablePanelGroup>

      {/* Expand button for collapsed chat panel */}
      {isChatCollapsed && (
        <Button
          onClick={handleExpandChat}
          className="absolute left-2 top-2 z-10"
          size="sm"
          variant="outline"
          aria-label="Expand chat panel"
        >
          <ChevronRight className="h-4 w-4 mr-1" />
          Chat
        </Button>
      )}
    </div>
  )
}
