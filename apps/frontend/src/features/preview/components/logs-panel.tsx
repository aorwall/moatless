import { ChevronDown, ChevronUp, Server, Terminal } from 'lucide-react'
import { useState } from 'react'
import { ServerLogsViewer } from '@/features/tasks/components/server-logs-viewer/server-logs-viewer'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/lib/components/ui/tabs'
import { cn } from '@/lib/utils'
import { ConsoleLogs } from './console-logs'

interface LogsPanelProps {
  className?: string
  onToggleExpanded?: () => void
  isExpanded?: boolean
}

export function LogsPanel({ className, onToggleExpanded, isExpanded = false }: LogsPanelProps) {
  const [activeTab, setActiveTab] = useState('server')

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
          {/* Expand/collapse button */}
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

          {/* Tabs on the right side of header */}
          <TabsList className="h-7">
            <TabsTrigger value="server" className="text-xs h-7">
              <Server className="h-3 w-3" />
              <span className="ml-1">Server</span>
            </TabsTrigger>
            <TabsTrigger value="console" className="text-xs h-7">
              <Terminal className="h-3 w-3" />
              <span className="ml-1">Console</span>
            </TabsTrigger>
          </TabsList>
        </div>
      </div>

      {/* Tab content - only show when expanded */}
      {isExpanded && (
        <div className="flex-1 overflow-hidden">
          <TabsContent value="server" className="h-full m-0 overflow-hidden">
            <ServerLogsViewer />
          </TabsContent>
          <TabsContent value="console" className="h-full m-0 overflow-hidden">
            <ConsoleLogs />
          </TabsContent>
        </div>
      )}
    </Tabs>
  )
}
