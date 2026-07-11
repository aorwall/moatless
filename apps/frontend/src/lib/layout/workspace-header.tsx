import type { ReactNode } from 'react'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { cn } from '@/lib/utils'

interface WorkspaceHeaderProps {
  /**
   * Left section — typically the workspace view tabs. Injected as a slot so this
   * chrome stays generic (no feature imports). Hidden on mobile when
   * `showToggle` is set.
   */
  leading?: ReactNode | undefined
  centerContent?: ReactNode | undefined
  rightContent?: ReactNode | undefined
  showToggle?: boolean | undefined
  className?: string | undefined
}

/**
 * Generic workspace header bar: a fixed-height row with a leading slot (view
 * tabs), a centered slot (contextual controls), and a right slot (actions).
 *
 * Lives in `lib/layout` — it composes only slots, so any feature can supply its
 * own tabs/controls without the header depending on that feature.
 */
export function WorkspaceHeader({
  leading,
  centerContent,
  rightContent,
  showToggle = true,
  className,
}: WorkspaceHeaderProps) {
  const isMobile = useIsMobile()

  return (
    <div className={cn('h-[58px] px-2 md:px-4 py-2 border-b flex-shrink-0', className)}>
      <div className="flex items-center h-full w-full gap-2">
        {/* Left section - View toggle (hidden on mobile). The leading slot
            (e.g. ViewTabs) includes Files + the always-available Sandbox tab
            even with no servers. */}
        {showToggle && !isMobile && leading}

        {/* Center section - centered within the free space between the tabs and
            the right content, so contextual controls read as centered without
            ever overlapping the view-toggle icons. */}
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
