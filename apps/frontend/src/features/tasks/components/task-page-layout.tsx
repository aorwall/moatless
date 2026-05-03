import { ChevronRight, Sparkles } from 'lucide-react'
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from 'react'
import type { ImperativePanelHandle } from 'react-resizable-panels'
import { Button } from '@/lib/components/ui/button'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { cn } from '@/lib/utils'

interface ChatToggleContextValue {
  isChatCollapsed: boolean
  toggleChat: () => void
}

const ChatToggleContext = createContext<ChatToggleContextValue | null>(null)

export function useChatToggle(): ChatToggleContextValue {
  const ctx = useContext(ChatToggleContext)
  if (!ctx) {
    throw new Error('useChatToggle must be used within TaskPageLayout')
  }
  return ctx
}

interface TaskPageLayoutProps {
  header: ReactNode
  chat: ReactNode
  workspacePanel: ReactNode
  isMobile?: boolean
  mobileView?: string
  className?: string
  kiosk?: boolean
  onChatToggleReady?: (toggle: () => void) => void
  onChatCollapsedChange?: (collapsed: boolean) => void
}

export function TaskPageLayout({
  header,
  chat,
  workspacePanel,
  isMobile = false,
  mobileView = 'chat',
  className,
  kiosk = false,
  onChatToggleReady,
  onChatCollapsedChange,
}: TaskPageLayoutProps) {
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

  const handleChatCollapse = useCallback(() => {
    setIsChatCollapsed(true)
    onChatCollapsedChange?.(true)
  }, [onChatCollapsedChange])

  const handleChatExpand = useCallback(() => {
    setIsChatCollapsed(false)
    onChatCollapsedChange?.(false)
  }, [onChatCollapsedChange])

  useEffect(() => {
    if (!onChatToggleReady) {
      return
    }
    onChatToggleReady(() => {
      const panel = chatPanelRef.current
      if (!panel) {
        return
      }
      if (panel.isCollapsed()) {
        panel.expand()
      } else {
        panel.collapse()
      }
    })
  }, [onChatToggleReady])

  // Mobile layout: show single panel based on mobileView
  if (isMobile) {
    return (
      <ChatToggleContext.Provider value={{ isChatCollapsed, toggleChat }}>
        <div className={cn('h-full w-full flex flex-col overflow-hidden', className)}>
          <div className="flex-1 min-h-0 relative">
            {header}
            {mobileView === 'chat' ? chat : workspacePanel}
          </div>
        </div>
      </ChatToggleContext.Provider>
    )
  }

  const chatPanel = (
    <ResizablePanel
      ref={chatPanelRef}
      defaultSize={40}
      minSize={10}
      maxSize={60}
      collapsible={true}
      collapsedSize={0}
      onCollapse={handleChatCollapse}
      onExpand={handleChatExpand}
      className="flex flex-col overflow-hidden"
    >
      <div className="flex-1 min-h-0 flex flex-col relative">
        {header}
        {chat}
      </div>
    </ResizablePanel>
  )

  const workPanel = (
    <ResizablePanel className="flex flex-col overflow-hidden">{workspacePanel}</ResizablePanel>
  )

  // Desktop layout: resizable panels
  return (
    <ChatToggleContext.Provider value={{ isChatCollapsed, toggleChat }}>
      <div className={cn('h-full w-full relative', className)}>
        <ResizablePanelGroup direction="horizontal" className="h-full w-full">
          {kiosk ? workPanel : chatPanel}
          <ResizableHandle
            withHandle
            className={cn(
              isChatCollapsed && 'bg-primary/20 w-1 hover:bg-primary/40 transition-colors',
            )}
          />
          {kiosk ? chatPanel : workPanel}
        </ResizablePanelGroup>

        {/* Expand button for collapsed chat panel */}
        {isChatCollapsed &&
          (kiosk ? (
            <Button
              onClick={handleExpandChat}
              className="absolute right-2 top-2 z-10"
              size="icon"
              variant="outline"
              aria-label="Expand chat panel"
            >
              <Sparkles className="h-4 w-4" />
            </Button>
          ) : (
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
          ))}
      </div>
    </ChatToggleContext.Provider>
  )
}
