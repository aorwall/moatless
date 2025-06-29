import { useParams } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { FileTree, FileViewer } from '@/features/file-viewer'
import { Preview, useServerStatus } from '@/features/preview'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { Tabs, TabsContent } from '@/lib/components/ui/tabs'
import { cn } from '@/lib/utils'
import { useTaskContext } from '../contexts/task-data-context'
import { ChatView } from './chat-view'
import { EventViewer } from './event-viewer'
import { GitDiffViewer } from './git-diff-viewer'
import { TaskDetailSkeleton } from './task-skeletons'

export function TaskPage() {
  const { rightPanelView, setRightPanelView, task, isLoading, error } = useTaskContext()
  const { taskId } = useParams({ from: '/tasks/$taskId' })
  const [leftPanelSize, setLeftPanelSize] = useState(60)
  const [rightPanelSize, setRightPanelSize] = useState(40)
  const [selectedFile, setSelectedFile] = useState<string>('')

  // All hooks at the top before any conditional logic
  const { data: serverStatus } = useServerStatus(taskId, {
    enabled: rightPanelView === 'server',
  })

  useEffect(() => {
    const handleStorageChange = () => {
      const savedLeftSize = localStorage.getItem('leftPanelSize')
      const savedRightSize = localStorage.getItem('rightPanelSize')
      if (savedLeftSize) setLeftPanelSize(Number(savedLeftSize))
      if (savedRightSize) setRightPanelSize(Number(savedRightSize))
    }

    handleStorageChange()
    window.addEventListener('storage', handleStorageChange)
    return () => window.removeEventListener('storage', handleStorageChange)
  }, [])

  if (error) {
    return <ErrorAlert error={error} />
  }

  if (isLoading) {
    return <TaskDetailSkeleton />
  }

  if (!task) {
    return <ErrorAlert error={new Error('Task not found')} />
  }

  return (
    <div className="flex flex-col h-full w-full absolute inset-0">
      <ResizablePanelGroup direction="horizontal" className="flex-1 w-full">
        {/* Left Panel - Chat */}
        <ResizablePanel
          defaultSize={leftPanelSize}
          minSize={30}
          maxSize={80}
          onResize={(size) => {
            setLeftPanelSize(size)
            localStorage.setItem('leftPanelSize', size.toString())
          }}
          className="flex flex-col"
        >
          <ErrorBoundary componentName="Chat View">
            <ChatView />
          </ErrorBoundary>
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Right Panel - Events/Files/Preview */}
        <ResizablePanel
          defaultSize={rightPanelSize}
          minSize={20}
          maxSize={70}
          onResize={(size) => {
            setRightPanelSize(size)
            localStorage.setItem('rightPanelSize', size.toString())
          }}
          className="flex flex-col"
        >
          <div className="border-b border-border">
            <Tabs
              value={rightPanelView}
              onValueChange={(value) => setRightPanelView(value as typeof rightPanelView)}
              className="w-full"
            >
              <div className="flex items-center justify-between px-4 py-2">
                <div className="flex space-x-4">
                  <button
                    type="button"
                    className={cn(
                      'px-3 py-1 rounded text-sm font-medium transition-colors',
                      rightPanelView === 'events'
                        ? 'bg-primary text-primary-foreground'
                        : 'text-muted-foreground hover:text-foreground',
                    )}
                    onClick={() => setRightPanelView('events')}
                  >
                    Events
                  </button>
                  <button
                    type="button"
                    className={cn(
                      'px-3 py-1 rounded text-sm font-medium transition-colors',
                      rightPanelView === 'diff'
                        ? 'bg-primary text-primary-foreground'
                        : 'text-muted-foreground hover:text-foreground',
                    )}
                    onClick={() => setRightPanelView('diff')}
                  >
                    Changes
                  </button>
                  <button
                    type="button"
                    className={cn(
                      'px-3 py-1 rounded text-sm font-medium transition-colors',
                      rightPanelView === 'files'
                        ? 'bg-primary text-primary-foreground'
                        : 'text-muted-foreground hover:text-foreground',
                    )}
                    onClick={() => setRightPanelView('files')}
                  >
                    Files
                  </button>
                  <button
                    type="button"
                    className={cn(
                      'px-3 py-1 rounded text-sm font-medium transition-colors',
                      rightPanelView === 'preview'
                        ? 'bg-primary text-primary-foreground'
                        : 'text-muted-foreground hover:text-foreground',
                    )}
                    onClick={() => setRightPanelView('preview')}
                  >
                    Preview
                  </button>
                  <button
                    type="button"
                    className={cn(
                      'px-3 py-1 rounded text-sm font-medium transition-colors',
                      rightPanelView === 'server'
                        ? 'bg-primary text-primary-foreground'
                        : 'text-muted-foreground hover:text-foreground',
                    )}
                    onClick={() => setRightPanelView('server')}
                  >
                    Server
                  </button>
                </div>
              </div>
            </Tabs>
          </div>

          <div className="flex-1 overflow-hidden">
            <Tabs value={rightPanelView} className="h-full">
              <TabsContent value="events" className="h-full mt-0">
                <ErrorBoundary componentName="Event Viewer">
                  <EventViewer />
                </ErrorBoundary>
              </TabsContent>
              <TabsContent value="diff" className="h-full mt-0">
                <ErrorBoundary componentName="Git Diff Viewer">
                  <GitDiffViewer />
                </ErrorBoundary>
              </TabsContent>
              <TabsContent value="files" className="h-full mt-0">
                <ErrorBoundary componentName="File Tree">
                  <div className="flex h-full">
                    <div className="w-80 border-r border-border">
                      <FileTree taskId={taskId} onFileSelect={setSelectedFile} />
                    </div>
                    <div className="flex-1">
                      <FileViewer taskId={taskId} filePath={selectedFile} />
                    </div>
                  </div>
                </ErrorBoundary>
              </TabsContent>
              <TabsContent value="preview" className="h-full mt-0">
                <ErrorBoundary componentName="Preview">
                  <Preview config={{ url: serverStatus?.url || '' }} />
                </ErrorBoundary>
              </TabsContent>
              <TabsContent value="server" className="h-full mt-0">
                <ErrorBoundary componentName="Server Status">
                  <div className="p-4">
                    <h3 className="text-lg font-medium mb-4">Server Status</h3>
                    {serverStatus ? (
                      <div className="space-y-2">
                        <div>Status: {serverStatus.running ? 'Running' : 'Stopped'}</div>
                        <div>Configured: {serverStatus.configured ? 'Yes' : 'No'}</div>
                        {serverStatus.url && <div>URL: {serverStatus.url}</div>}
                        {serverStatus.pid && <div>PID: {serverStatus.pid}</div>}
                        <div>Uptime: {serverStatus.uptimeSeconds}s</div>
                      </div>
                    ) : (
                      <div>Loading server status...</div>
                    )}
                  </div>
                </ErrorBoundary>
              </TabsContent>
            </Tabs>
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}
