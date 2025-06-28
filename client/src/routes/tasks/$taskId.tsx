import { createFileRoute } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { FileTree, FileViewer } from '@/features/file-viewer'
import { Preview } from '@/features/preview'
import { ChatView } from '@/features/tasks/components/chat-view'
import { EventViewer } from '@/features/tasks/components/event-viewer'
import { GitDiffViewer } from '@/features/tasks/components/git-diff-viewer'
import { ServerLogsViewer } from '@/features/tasks/components/server-logs-viewer'
import { TaskDetailSkeleton } from '@/features/tasks/components/task-skeletons'
import { useServerStatus, useTask } from '@/features/tasks/hooks/use-tasks'
import { useRightPanelView, useTaskUIStore } from '@/features/tasks/stores/task-ui-store'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { Tabs, TabsContent } from '@/lib/components/ui/tabs'
import { cn } from '@/lib/utils'

export const Route = createFileRoute('/tasks/$taskId')({
  component: TaskDetail,
})

function TaskDetail() {
  const { taskId } = Route.useParams()
  const { data: metadata, isLoading, error } = useTask(taskId)
  const { data: serverStatus } = useServerStatus(taskId)
  const rightPanelView = useRightPanelView()
  const setRightPanelView = useTaskUIStore((state) => state.setRightPanelView)
  const setSelectedTaskId = useTaskUIStore((state) => state.setSelectedTaskId)
  const resetUIState = useTaskUIStore((state) => state.resetUIState)
  const [selectedFile, setSelectedFile] = useState<string | null>(null)

  // Set selected task ID and reset UI state when component unmounts or task changes
  useEffect(() => {
    setSelectedTaskId(taskId)
    return () => {
      resetUIState()
    }
  }, [taskId, setSelectedTaskId, resetUIState])

  // If server or preview tab is selected but server is not configured, switch to events tab
  useEffect(() => {
    if (
      (rightPanelView === 'server' || rightPanelView === 'preview') &&
      !serverStatus?.configured
    ) {
      setRightPanelView('events')
    }
  }, [rightPanelView, serverStatus?.configured, setRightPanelView])

  if (error) {
    return (
      <div className="min-h-screen flex items-center justify-center p-4">
        <ErrorAlert error={error} title="Failed to load task" className="max-w-md w-full" />
      </div>
    )
  }

  if (isLoading) {
    return <TaskDetailSkeleton />
  }

  if (!metadata) {
    return (
      <div className="min-h-screen flex items-center justify-center p-4">
        <ErrorAlert
          error={new Error(`Task with ID ${taskId} could not be found`)}
          title="Task not found"
          className="max-w-md w-full"
        />
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full w-full absolute inset-0">
      <ResizablePanelGroup direction="horizontal" className="flex-1 w-full">
        {/* Left side - Chat View */}
        <ResizablePanel defaultSize={50} minSize={10}>
          <div className="h-full w-full">
            <ErrorBoundary componentName="Chat View">
              <ChatView />
            </ErrorBoundary>
          </div>
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Right side - Tabs with Events and Diff */}
        <ResizablePanel defaultSize={50} minSize={20}>
          <div className="h-full flex flex-col">
            <Tabs
              value={rightPanelView}
              onValueChange={(value) =>
                setRightPanelView(value as 'events' | 'diff' | 'server' | 'preview' | 'files')
              }
              className="h-full w-full flex flex-col"
            >
              <div className="h-[58px] px-4 py-2 border-b flex items-center">
                <div className="relative flex w-fit min-w-0 flex-1 items-center gap-2 overflow-x-auto">
                  <button
                    onClick={() => setRightPanelView('events')}
                    className={cn(
                      'font-regular group h-7 max-w-56 select-none whitespace-nowrap rounded-md px-2 text-sm font-medium transition-all cursor-pointer',
                      rightPanelView === 'events'
                        ? 'bg-black/10 dark:bg-white/10 text-gray-900 dark:text-gray-100'
                        : 'hover:bg-black/5 dark:hover:bg-white/5 bg-transparent text-gray-500 dark:text-gray-400',
                    )}
                    data-testid="tab-file-button"
                    data-active-tab={rightPanelView === 'events'}
                  >
                    <div className="truncate">
                      <div className="flex items-center gap-2">
                        <span>Events</span>
                      </div>
                    </div>
                  </button>
                  <button
                    onClick={() => setRightPanelView('diff')}
                    className={cn(
                      'font-regular group h-7 max-w-56 select-none whitespace-nowrap rounded-md px-2 text-sm font-medium transition-all cursor-pointer',
                      rightPanelView === 'diff'
                        ? 'bg-black/10 dark:bg-white/10 text-gray-900 dark:text-gray-100'
                        : 'hover:bg-black/5 dark:hover:bg-white/5 bg-transparent text-gray-500 dark:text-gray-400',
                    )}
                    data-testid="tab-file-button"
                    data-active-tab={rightPanelView === 'diff'}
                  >
                    <div className="truncate">
                      <div className="flex items-center gap-2">
                        <span>Diff</span>
                      </div>
                    </div>
                  </button>
                  <button
                    onClick={() => setRightPanelView('files')}
                    className={cn(
                      'font-regular group h-7 max-w-56 select-none whitespace-nowrap rounded-md px-2 text-sm font-medium transition-all cursor-pointer',
                      rightPanelView === 'files'
                        ? 'bg-black/10 dark:bg-white/10 text-gray-900 dark:text-gray-100'
                        : 'hover:bg-black/5 dark:hover:bg-white/5 bg-transparent text-gray-500 dark:text-gray-400',
                    )}
                    data-testid="tab-file-button"
                    data-active-tab={rightPanelView === 'files'}
                  >
                    <div className="truncate">
                      <div className="flex items-center gap-2">
                        <span>Files</span>
                      </div>
                    </div>
                  </button>
                  {serverStatus?.configured && (
                    <button
                      onClick={() => setRightPanelView('preview')}
                      className={cn(
                        'font-regular group h-7 max-w-56 select-none whitespace-nowrap rounded-md px-2 text-sm font-medium transition-all cursor-pointer',
                        rightPanelView === 'preview'
                          ? 'bg-black/10 dark:bg-white/10 text-gray-900 dark:text-gray-100'
                          : 'hover:bg-black/5 dark:hover:bg-white/5 bg-transparent text-gray-500 dark:text-gray-400',
                      )}
                      data-testid="tab-file-button"
                      data-active-tab={rightPanelView === 'preview'}
                    >
                      <div className="truncate">
                        <div className="flex items-center gap-2">
                          <span>Preview</span>
                        </div>
                      </div>
                    </button>
                  )}
                </div>
              </div>
              <TabsContent value="events" className="flex-1 w-full overflow-hidden mt-0">
                <ErrorBoundary componentName="Event Viewer">
                  <EventViewer />
                </ErrorBoundary>
              </TabsContent>
              <TabsContent value="diff" className="flex-1 w-full overflow-hidden mt-0">
                <ErrorBoundary componentName="Git Diff Viewer">
                  <GitDiffViewer />
                </ErrorBoundary>
              </TabsContent>
              {serverStatus?.configured && (
                <>
                  <TabsContent value="server" className="flex-1 w-full overflow-hidden mt-0">
                    <ErrorBoundary componentName="Server Logs Viewer">
                      <ServerLogsViewer />
                    </ErrorBoundary>
                  </TabsContent>
                  <TabsContent value="preview" className="flex-1 w-full overflow-hidden mt-0">
                    <ErrorBoundary componentName="Preview">
                      <Preview
                        config={{
                          url: serverStatus.url || '',
                        }}
                        serverStatus={serverStatus}
                        onElementSelected={(element) => {
                          if (element) {
                            console.log('Selected element:', element.selector)
                          }
                        }}
                        className="h-full"
                      />
                    </ErrorBoundary>
                  </TabsContent>
                </>
              )}
              <TabsContent value="files" className="flex-1 w-full overflow-hidden mt-0">
                <ErrorBoundary componentName="File Viewer">
                  <ResizablePanelGroup direction="horizontal" className="h-full">
                    <ResizablePanel defaultSize={30} minSize={20}>
                      <div className="h-full border-r">
                        <FileTree taskId={taskId} onFileSelect={setSelectedFile} />
                      </div>
                    </ResizablePanel>
                    <ResizableHandle />
                    <ResizablePanel defaultSize={70}>
                      {selectedFile ? (
                        <FileViewer taskId={taskId} filePath={selectedFile} />
                      ) : (
                        <div className="flex items-center justify-center h-full text-muted-foreground">
                          Select a file to view
                        </div>
                      )}
                    </ResizablePanel>
                  </ResizablePanelGroup>
                </ErrorBoundary>
              </TabsContent>
            </Tabs>
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}
