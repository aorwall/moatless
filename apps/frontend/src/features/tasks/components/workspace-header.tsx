import type { ReactNode } from 'react'
import { ServerTabs } from '@/features/preview/components/server-tabs'
import type { ServerEntry } from '@/features/preview/hooks/use-servers'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { cn } from '@/lib/utils'

interface WorkspaceHeaderProps {
  activeView: string
  onViewChange: (view: string) => void
  centerContent?: ReactNode
  rightContent?: ReactNode
  showToggle?: boolean
  /** Dynamic server list — renders ServerTabs when provided */
  servers?: ServerEntry[]
  className?: string
}

export function WorkspaceHeader({
  activeView,
  onViewChange,
  centerContent,
  rightContent,
  showToggle = true,
  servers,
  className,
}: WorkspaceHeaderProps) {
  const isMobile = useIsMobile()

  return (
    <div className={cn('h-[58px] px-2 md:px-4 py-2 border-b flex-shrink-0', className)}>
      <div className="relative flex items-center h-full w-full gap-2">
        {/* Left section - View toggle (hide on mobile, show on desktop when servers exist) */}
        {showToggle && !isMobile && servers && servers.length > 0 && (
          <ServerTabs servers={servers} activeView={activeView} onViewChange={onViewChange} />
        )}

        {/* Center section - Absolutely centered */}
        <div className="absolute left-1/2 -translate-x-1/2 flex items-center">{centerContent}</div>

        {/* Right section */}
        {rightContent && (
          <div className="ml-auto flex items-center justify-end gap-2 flex-shrink-0">
            {rightContent}
          </div>
        )}
      </div>
    </div>
  )
}
