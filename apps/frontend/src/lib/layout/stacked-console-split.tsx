import type { ReactNode } from 'react'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { cn } from '@/lib/utils'

interface StackedConsoleSplitProps {
  /** Main content rendered in the top (flexible) panel. */
  children: ReactNode
  /**
   * Renders the console. Receives whether the console is currently expanded so
   * the same console component can adapt (e.g. show a full log view vs. a
   * collapsed toolbar).
   */
  renderConsole: (isExpanded: boolean) => ReactNode
  /** Whether the console is expanded (draggable panel) or collapsed (fixed bar). */
  isConsoleExpanded: boolean
  /** Hide the console entirely — no expanded panel, no collapsed bar. */
  hideConsole?: boolean
  /** Expanded console panel size, as a percent of the split. */
  consoleDefaultSize?: number
  consoleMinSize?: number
  consoleMaxSize?: number
  /**
   * Optional slot rendered between the panel group and the collapsed console bar
   * (used by the preview for its resource-metrics strip).
   */
  footer?: ReactNode
  /** Extra classes for the inner `ResizablePanelGroup`. */
  groupClassName?: string
  /** Extra classes for the top content `ResizablePanel`. */
  contentClassName?: string
}

/**
 * Vertical split with a collapsible console pinned to the bottom.
 *
 * When the console is expanded it lives in a draggable `ResizablePanel`; when
 * collapsed it shrinks to a fixed-height bar so its toggle stays reachable.
 * Shared by the Preview and the Sandbox workspace panel, which both stack a
 * primary region above the same server-log console.
 */
export function StackedConsoleSplit({
  children,
  renderConsole,
  isConsoleExpanded,
  hideConsole = false,
  consoleDefaultSize = 20,
  consoleMinSize = 5,
  consoleMaxSize = 70,
  footer,
  groupClassName,
  contentClassName,
}: StackedConsoleSplitProps) {
  const showExpandedConsole = !hideConsole && isConsoleExpanded
  const showCollapsedConsole = !(hideConsole || isConsoleExpanded)

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <ResizablePanelGroup
        direction="vertical"
        className={cn('flex-1 min-h-0 overflow-hidden', groupClassName)}
      >
        <ResizablePanel className={cn('overflow-hidden', contentClassName)}>
          {children}
        </ResizablePanel>

        {showExpandedConsole && <ResizableHandle withHandle />}
        {showExpandedConsole && (
          <ResizablePanel
            defaultSize={consoleDefaultSize}
            minSize={consoleMinSize}
            maxSize={consoleMaxSize}
          >
            {renderConsole(true)}
          </ResizablePanel>
        )}
      </ResizablePanelGroup>

      {footer}

      {showCollapsedConsole && (
        <div className="h-10 flex-shrink-0 border-t">{renderConsole(false)}</div>
      )}
    </div>
  )
}
