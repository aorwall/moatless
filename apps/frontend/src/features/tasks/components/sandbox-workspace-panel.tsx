import type { ServerEntry } from '@moatless/api-client'
import { useState } from 'react'
import { LogsPanel } from '@/features/preview/components/logs-panel'
import type { SandboxEvent } from '@/features/preview/types'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { TaskControlPanel } from './task-control-panel'
import { WorkspaceHeader } from './workspace-header'

interface SandboxWorkspacePanelProps {
  view: string
  onViewChange: (view: string) => void
  servers?: ServerEntry[]
  sandboxEvents?: SandboxEvent[]
}

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
  sandboxEvents = [],
}: SandboxWorkspacePanelProps) {
  const [isConsoleExpanded, setIsConsoleExpanded] = useState(true)
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
      <WorkspaceHeader activeView={view} onViewChange={onViewChange} servers={servers} />
      <div className="flex-1 min-h-0 flex flex-col">
        <ResizablePanelGroup direction="vertical" className="flex-1 min-h-0 overflow-hidden">
          <ResizablePanel className="overflow-hidden">
            <TaskControlPanel className="h-full" />
          </ResizablePanel>

          {isConsoleExpanded && <ResizableHandle withHandle />}
          {isConsoleExpanded && (
            <ResizablePanel defaultSize={30} minSize={10} maxSize={70}>
              {renderConsole(true)}
            </ResizablePanel>
          )}
        </ResizablePanelGroup>

        {!isConsoleExpanded && (
          <div className="h-10 flex-shrink-0 border-t">{renderConsole(false)}</div>
        )}
      </div>
    </div>
  )
}
