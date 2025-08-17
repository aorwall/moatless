import type { ReactNode } from 'react'
import { usePanelView } from '@/features/tasks/contexts/task-ui-provider'
import { cn } from '@/lib/utils'

interface WorkspaceHeaderProps {
  centerContent?: ReactNode
  rightContent?: ReactNode
  className?: string
}

export function WorkspaceHeader({ centerContent, rightContent, className }: WorkspaceHeaderProps) {
  const { rightPanelView, setRightPanelView } = usePanelView()

  return (
    <div className={cn('h-[58px] px-4 py-2 border-b flex-shrink-0', className)}>
      <div className="grid grid-cols-[auto_1fr_auto] items-center h-full w-full">
        {/* Left section - View toggle */}
        <div className="flex items-center border rounded-md bg-muted/30">
          <button
            type="button"
            className={cn(
              'px-3 py-1 text-sm font-medium transition-colors rounded-l-md',
              rightPanelView === 'preview'
                ? 'bg-background text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground',
            )}
            onClick={() => setRightPanelView('preview')}
          >
            Preview
          </button>
          <button
            type="button"
            className={cn(
              'px-3 py-1 text-sm font-medium transition-colors rounded-r-md',
              rightPanelView === 'files'
                ? 'bg-background text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground',
            )}
            onClick={() => setRightPanelView('files')}
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
