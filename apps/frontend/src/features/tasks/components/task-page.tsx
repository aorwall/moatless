import { useParams } from '@tanstack/react-router'
import React, { useEffect } from 'react'
import { FileTree, FileViewer } from '@/features/file-viewer'
import { Preview, useServerStatus } from '@/features/preview'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { Tabs, TabsContent } from '@/lib/components/ui/tabs'
import { cn } from '@/lib/utils'
import {
  usePanelSizes,
  usePanelView,
  useSelectedFile,
  useTaskData,
  useTaskMetadata,
} from '../contexts/task-providers'
import { ChatView } from './chat-view'
import { EventViewer } from './event-viewer'
import { GitDiffViewer } from './git-diff-viewer'
import { TaskDetailSkeleton } from './task-skeletons'

function TaskPageComponent() {
  const { taskId } = useParams({ from: '/tasks/$taskId' })

  // Use specialized hooks that only cause re-renders when specific data changes
  const { isLoading, error } = useTaskData() // Only re-renders on loading/error changes
  useTaskMetadata() // Get task metadata for repository ID
  const { rightPanelView, setRightPanelView } = usePanelView() // Only re-renders on panel view changes
  const { panelSizes, setPanelSizes } = usePanelSizes() // Only re-renders on panel size changes
  const { selectedFile, setSelectedFile } = useSelectedFile() // Only re-renders on file selection changes

  // All hooks at the top before any conditional logic
  const { data: serverStatus } = useServerStatus(taskId, {
    enabled: rightPanelView === 'server',
  })

  useEffect(() => {
    // FIXED: Restore panel sizes from localStorage on mount only
    const restorePanelSizes = () => {
      const savedLeftSize = localStorage.getItem('leftPanelSize')
      const savedRightSize = localStorage.getItem('rightPanelSize')

      if (savedLeftSize || savedRightSize) {
        setPanelSizes({
          left: savedLeftSize ? Number(savedLeftSize) : 60, // Default to 60 if not saved
          right: savedRightSize ? Number(savedRightSize) : 40, // Default to 40 if not saved
        })
      }
    }

    // Handle storage changes from other tabs
    const handleStorageChange = (e: StorageEvent) => {
      if (e.key === 'leftPanelSize' || e.key === 'rightPanelSize') {
        restorePanelSizes()
      }
    }

    // FIXED: Only restore on mount - no stale closure issues
    restorePanelSizes()

    // Listen for storage changes from other tabs
    window.addEventListener('storage', handleStorageChange)
    return () => window.removeEventListener('storage', handleStorageChange)
  }, [setPanelSizes]) // Only depend on setPanelSizes which is stable

  if (error) {
    return <ErrorAlert error={error} />
  }

  if (isLoading) {
    return <TaskDetailSkeleton />
  }

  return (
    <div className="flex flex-col h-full w-full absolute inset-0">
      <ResizablePanelGroup direction="horizontal" className="flex-1 w-full">
        {/* Left Panel - Chat */}
        <ResizablePanel
          defaultSize={panelSizes.left}
          minSize={30}
          maxSize={80}
          onResize={(size) => {
            setPanelSizes({ left: size, right: panelSizes.right })
            localStorage.setItem('leftPanelSize', size.toString())
          }}
          className="flex flex-col"
        >
          <ChatView />
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Right Panel - Events/Files/Preview */}
        <ResizablePanel
          defaultSize={panelSizes.right}
          minSize={20}
          maxSize={70}
          onResize={(size) => {
            setPanelSizes({ left: panelSizes.left, right: size })
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
                  <ResizablePanelGroup direction="horizontal" className="h-full">
                    <ResizablePanel defaultSize={25} minSize={15} maxSize={50}>
                      <FileTree taskId={taskId} onFileSelect={setSelectedFile} />
                    </ResizablePanel>
                    <ResizableHandle withHandle />
                    <ResizablePanel defaultSize={75}>
                      <FileViewer taskId={taskId} filePath={selectedFile} />
                    </ResizablePanel>
                  </ResizablePanelGroup>
                </ErrorBoundary>
              </TabsContent>
              <TabsContent value="preview" className="h-full mt-0">
                <ErrorBoundary componentName="Preview">
                  <Preview config={{ url: serverStatus?.url || '' }} serverStatus={serverStatus} />
                </ErrorBoundary>
              </TabsContent>
            </Tabs>
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}

export const TaskPage = React.memo(TaskPageComponent)
