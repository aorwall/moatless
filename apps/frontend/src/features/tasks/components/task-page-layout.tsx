import type { ReactNode } from 'react'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { cn } from '@/lib/utils'

interface TaskPageLayoutProps {
  header: ReactNode
  chat: ReactNode
  workspacePanel: ReactNode
  className?: string
}

export function TaskPageLayout({ header, chat, workspacePanel, className }: TaskPageLayoutProps) {
  return (
    <div className={cn('h-full w-full', className)}>
      <ResizablePanelGroup direction="horizontal" className="h-full w-full">
        <ResizablePanel defaultSize={40} minSize={25} maxSize={60} className="flex flex-col">
          {header}
          <div className="flex-1 min-h-0 flex flex-col overflow-hidden">{chat}</div>
        </ResizablePanel>
        <ResizableHandle withHandle />
        <ResizablePanel className="flex flex-col overflow-hidden">{workspacePanel}</ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}
