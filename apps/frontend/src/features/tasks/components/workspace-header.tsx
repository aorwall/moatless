import type { ReactNode } from 'react'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { cn } from '@/lib/utils'

interface WorkspaceHeaderProps {
  activeView: 'files' | 'preview'
  onViewChange: (view: 'files' | 'preview') => void
  centerContent?: ReactNode
  rightContent?: ReactNode
  showToggle?: boolean
  className?: string
}

export function WorkspaceHeader({
  activeView,
  onViewChange,
  centerContent,
  rightContent,
  showToggle = true,
  className,
}: WorkspaceHeaderProps) {
  const isMobile = useIsMobile()

  return (
    <div className={cn('h-[58px] px-2 md:px-4 py-2 border-b flex-shrink-0', className)}>
      <div className="flex items-center h-full w-full gap-2">
        {/* Left section - View toggle (hide on mobile, show on desktop if serverManagement is true) */}
        {showToggle && !isMobile && (
          <div className="flex items-center border rounded-md bg-muted/30 flex-shrink-0">
            <button
              type="button"
              className={cn(
                'px-3 py-1 text-sm font-medium transition-colors rounded-l-md',
                activeView === 'preview'
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground',
              )}
              onClick={() => onViewChange('preview')}
            >
              Preview
            </button>
            <button
              type="button"
              className={cn(
                'px-3 py-1 text-sm font-medium transition-colors rounded-r-md',
                activeView === 'files'
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground',
              )}
              onClick={() => onViewChange('files')}
            >
              Files
            </button>
          </div>
        )}

        {/* Center section - Takes up available space */}
        <div className="flex items-center justify-center flex-1 min-w-0">{centerContent}</div>

        {/* Right section */}
        {rightContent && (
          <div className="flex items-center justify-end gap-2 flex-shrink-0">{rightContent}</div>
        )}
      </div>
    </div>
  )
}
