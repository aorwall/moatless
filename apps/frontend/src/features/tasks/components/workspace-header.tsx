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
    <div className={cn('h-[58px] px-4 py-2 border-b flex-shrink-0', className)}>
      <div className="grid grid-cols-[auto_1fr_auto] items-center h-full w-full">
        {/* Left section - View toggle (hide on mobile, show on desktop if serverManagement is true) */}
        {showToggle && !isMobile ? (
          <div className="flex items-center border rounded-md bg-muted/30">
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
        ) : (
          <div />
        )}

        {/* Center section */}
        <div className="flex items-center justify-center">{centerContent}</div>

        {/* Right section */}
        <div className="flex items-center justify-end gap-2">{rightContent}</div>
      </div>
    </div>
  )
}
