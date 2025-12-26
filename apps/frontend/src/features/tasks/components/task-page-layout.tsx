import { ChevronRight } from 'lucide-react'
import type { ReactNode } from 'react'
import { useRef, useState } from 'react'
import type { ImperativePanelHandle } from 'react-resizable-panels'
import type { TaskView } from '@/features/tasks/hooks/use-task-view'
import { Button } from '@/lib/components/ui/button'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { cn } from '@/lib/utils'

interface TaskPageLayoutProps {
  header: ReactNode
  chat: ReactNode
  workspacePanel: ReactNode
  isMobile?: boolean
  mobileView?: TaskView
  className?: string
}

export function TaskPageLayout({
  header,
  chat,
  workspacePanel,
  isMobile = false,
  mobileView = 'chat',
  className,
}: TaskPageLayoutProps) {
  const chatPanelRef = useRef<ImperativePanelHandle>(null)
  const [isChatCollapsed, setIsChatCollapsed] = useState(false)

  const handleExpandChat = () => {
    chatPanelRef.current?.expand()
  }

  // Mobile layout: show single panel based on mobileView
  if (isMobile) {
    return (
      <div className={cn('h-full w-full flex flex-col overflow-hidden', className)}>
        <div className="flex-1 min-h-0 relative">
          {header}
          {mobileView === 'chat' ? chat : workspacePanel}
        </div>
      </div>
    )
  }

  // Desktop layout: resizable panels
  return (
    <div className={cn('h-full w-full relative', className)}>
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
            {header}
            {chat}
          </div>
        </ResizablePanel>
        <ResizableHandle
          withHandle
          className={cn(
            isChatCollapsed && 'bg-primary/20 w-1 hover:bg-primary/40 transition-colors',
          )}
        />
        <ResizablePanel className="flex flex-col overflow-hidden">{workspacePanel}</ResizablePanel>
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
