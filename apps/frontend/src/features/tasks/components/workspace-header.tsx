import type { ServerEntry } from '@moatless/api-client'
import type { ReactNode } from 'react'
import { ServerTabs } from '@/features/preview/components/server-tabs'
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
      <div className="flex items-center h-full w-full gap-2">
        {/* Left section - View toggle (hidden on mobile). Always rendered on desktop:
            ServerTabs includes Files + the always-available Sandbox tab even with no servers. */}
        {showToggle && !isMobile && (
          <ServerTabs servers={servers ?? []} activeView={activeView} onViewChange={onViewChange} />
        )}

        {/* Center section - centered within the free space between the tabs and the
            right content, so the preview controls read as centered without ever
            overlapping the view-toggle icons. */}
        {centerContent && (
          <div className="flex-1 min-w-0 flex items-center gap-2 justify-center">
            {centerContent}
          </div>
        )}

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
