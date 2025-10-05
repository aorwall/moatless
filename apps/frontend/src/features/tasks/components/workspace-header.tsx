import type { ReactNode } from 'react'
import { usePanelView } from '@/features/tasks/contexts/task-ui-provider'
import { cn } from '@/lib/utils'

interface WorkspaceHeaderProps {
  activeView?: 'files' | 'preview'
  onViewChange?: (view: 'files' | 'preview') => void
  centerContent?: ReactNode
  rightContent?: ReactNode
  className?: string
}

export function WorkspaceHeader({
  activeView,
  onViewChange,
  centerContent,
  rightContent,
  className,
}: WorkspaceHeaderProps) {
  const context = usePanelView()
  const currentView = activeView ?? context.rightPanelView
  const changeView = onViewChange ?? context.setRightPanelView

  return (
    <div className={cn('h-[58px] px-4 py-2 border-b flex-shrink-0', className)}>
      <div className="grid grid-cols-[auto_1fr_auto] items-center h-full w-full">
        {/* Left section - View toggle */}
        <div className="flex items-center border rounded-md bg-muted/30">
          <button
            type="button"
            className={cn(
              'px-3 py-1 text-sm font-medium transition-colors rounded-l-md',
              currentView === 'preview'
                ? 'bg-background text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground',
            )}
            onClick={() => changeView('preview')}
          >
            Preview
          </button>
          <button
            type="button"
            className={cn(
              'px-3 py-1 text-sm font-medium transition-colors rounded-r-md',
              currentView === 'files'
                ? 'bg-background text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground',
            )}
            onClick={() => changeView('files')}
          >
            Files
          </button>
        </div>

        {/* Center section */}
        <div className="flex items-center justify-center">{centerContent}</div>

        {/* Right section */}
        <div className="flex items-center justify-end gap-2">{rightContent}</div>
      </div>
    </div>
  )
}
