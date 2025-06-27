import { Server, Terminal } from 'lucide-react'
import { useState } from 'react'
import { ServerLogsViewer } from '@/features/tasks/components/server-logs-viewer/server-logs-viewer'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/lib/components/ui/tabs'
import { cn } from '@/lib/utils'
import { ConsoleLogs } from './console-logs'

interface LogsPanelProps {
  className?: string
}

export function LogsPanel({ className }: LogsPanelProps) {
  const [activeTab, setActiveTab] = useState('server')

  return (
    <div className={cn('h-full border-t bg-background', className)}>
      <Tabs value={activeTab} onValueChange={setActiveTab} className="h-full flex flex-col">
        <div className="border-b px-4 py-2">
          <TabsList className="h-8">
            <TabsTrigger value="server" className="text-xs">
              <Server className="h-3 w-3" />
              <span className="ml-1">Server</span>
            </TabsTrigger>
            <TabsTrigger value="console" className="text-xs">
              <Terminal className="h-3 w-3" />
              <span className="ml-1">Console</span>
            </TabsTrigger>
          </TabsList>
        </div>
        <TabsContent value="server" className="flex-1 m-0">
          <ServerLogsViewer />
        </TabsContent>
        <TabsContent value="console" className="flex-1 m-0">
          <ConsoleLogs />
        </TabsContent>
      </Tabs>
    </div>
  )
}
