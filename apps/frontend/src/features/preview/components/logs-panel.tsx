import { Activity, ChevronDown, ChevronUp, Server, Terminal } from 'lucide-react'
import { useState } from 'react'
import { ServerLogsViewer } from '@/features/tasks/components/server-logs-viewer/server-logs-viewer'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/lib/components/ui/tabs'
import { cn } from '@/lib/utils'
import type { SandboxEvent } from '../types'
import { ConsoleLogs } from './console-logs'
import { EventLog } from './event-log'

interface ConsoleLog {
  id: string
  timestamp: string
  level: 'log' | 'info' | 'warn' | 'error' | 'debug'
  message: string
  args?: unknown[] | undefined
  stack?: string | undefined
}

interface LogsPanelProps {
  className?: string | undefined
  onToggleExpanded?: (() => void) | undefined
  isExpanded?: boolean | undefined
  consoleLogs?: ConsoleLog[] | undefined
  /** Server name for log streaming */
  serverName?: string | undefined
  /** Whether to show the Console tab (only for servers with iframe console) */
  showConsoleTab?: boolean | undefined
  /** Sandbox and server lifecycle events */
  sandboxEvents?: SandboxEvent[] | undefined
}

export function LogsPanel({
  className,
  onToggleExpanded,
  isExpanded = false,
  consoleLogs = [],
  serverName,
  showConsoleTab = true,
  sandboxEvents = [],
}: LogsPanelProps) {
  // Events are the default tab: sandbox/server lifecycle is what users most
  // often need when the console is auto-expanded during startup/failure.
  const [activeTab, setActiveTab] = useState('events')

  return (
    <Tabs
      value={activeTab}
      onValueChange={setActiveTab}
      className={cn('flex flex-col h-full bg-background border-t', className)}
    >
      {/* Header bar with title on left, expand/collapse in middle, tabs on right */}
      <div className="flex items-center justify-between px-4 border-b flex-shrink-0 h-10">
        <div className="flex items-center gap-2">
          <Terminal className="h-4 w-4" />
          <span className="text-sm font-medium">Console</span>
        </div>

        <div className="flex items-center gap-2">
          {/* Tabs - only show when expanded */}
          {isExpanded && (
            <TabsList className="h-7">
              <TabsTrigger value="server" className="text-xs h-7">
                <Server className="h-3 w-3" />
                <span className="ml-1">Server</span>
              </TabsTrigger>
              {showConsoleTab && (
                <TabsTrigger value="console" className="text-xs h-7">
                  <Terminal className="h-3 w-3" />
                  <span className="ml-1">Console</span>
                </TabsTrigger>
              )}
              <TabsTrigger value="events" className="text-xs h-7">
                <Activity className="h-3 w-3" />
                <span className="ml-1">Events</span>
              </TabsTrigger>
            </TabsList>
          )}

          {/* Expand/collapse button - to the right of tabs */}
          {onToggleExpanded && (
            <button
              type="button"
              onClick={onToggleExpanded}
              className="flex items-center justify-center w-6 h-6 rounded hover:bg-muted transition-colors"
              title={isExpanded ? 'Collapse console' : 'Expand console'}
            >
              {isExpanded ? <ChevronDown className="h-4 w-4" /> : <ChevronUp className="h-4 w-4" />}
            </button>
          )}
        </div>
      </div>

      {/* Tab content - only show when expanded */}
      {isExpanded && (
        <div className="flex-1 overflow-hidden">
          <TabsContent value="server" className="h-full m-0 overflow-hidden">
            <ServerLogsViewer serverName={serverName} />
          </TabsContent>
          {showConsoleTab && (
            <TabsContent value="console" className="h-full m-0 overflow-hidden">
              <ConsoleLogs logs={consoleLogs} />
            </TabsContent>
          )}
          <TabsContent value="events" className="h-full m-0 overflow-hidden">
            <EventLog events={sandboxEvents} />
          </TabsContent>
        </div>
      )}
    </Tabs>
  )
}
