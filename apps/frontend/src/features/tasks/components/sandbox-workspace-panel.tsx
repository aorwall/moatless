import type { ServerEntry } from '@moatless/api-client'
import { useState } from 'react'
import type { LifecycleEvent } from '@/features/events'
import { LogsPanel } from '@/features/preview/components/logs-panel'
import { ViewTabs } from '@/features/preview/components/view-tabs'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { StackedConsoleSplit } from '@/lib/layout/stacked-console-split'
import { WorkspaceHeader } from '@/lib/layout/workspace-header'
import { TaskControlPanel } from './task-control-panel'

interface SandboxWorkspacePanelProps {
  view: string
  onViewChange: (view: string) => void
  servers?: ServerEntry[] | undefined
  sandboxEvents?: LifecycleEvent[] | undefined
}

const EMPTY_SANDBOX_EVENTS: LifecycleEvent[] = []

/**
 * Right-pane "Sandbox" view: the control panel on top and the shared server-log
 * console (Events tab) at the bottom. The console lives inside the workspace
 * panel here — mirroring the Preview layout — so it stays scoped to the right
 * pane rather than spanning below both chat and the workspace.
 */
export function SandboxWorkspacePanel({
  view,
  onViewChange,
  servers,
  sandboxEvents = EMPTY_SANDBOX_EVENTS,
}: SandboxWorkspacePanelProps) {
  const [isConsoleExpanded, setIsConsoleExpanded] = useState(true)
  const isMobile = useIsMobile()
  const defaultServer = servers?.find((s) => s.default) ?? servers?.[0]

  const renderConsole = (isExpanded: boolean) => (
    <LogsPanel
      className="h-full"
      isExpanded={isExpanded}
      onToggleExpanded={() => setIsConsoleExpanded((prev) => !prev)}
      serverName={defaultServer?.name}
      // No iframe on the controls view — only Server + Events tabs are relevant.
      showConsoleTab={false}
      sandboxEvents={sandboxEvents}
    />
  )

  return (
    <div className="h-full flex flex-col">
      {/* Mobile uses the floating overlay's view switcher instead of this bar. */}
      {!isMobile && (
        <WorkspaceHeader
          leading={
            <ViewTabs servers={servers ?? []} activeView={view} onViewChange={onViewChange} />
          }
        />
      )}
      <StackedConsoleSplit
        renderConsole={renderConsole}
        isConsoleExpanded={isConsoleExpanded}
        consoleDefaultSize={30}
        consoleMinSize={10}
      >
        <TaskControlPanel className="h-full" />
      </StackedConsoleSplit>
    </div>
  )
}
