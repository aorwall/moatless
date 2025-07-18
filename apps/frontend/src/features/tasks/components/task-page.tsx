import { useParams } from '@tanstack/react-router'
import React from 'react'
import { FileExplorer } from '@/features/file-viewer'
import { Preview, useServerStatus } from '@/features/preview'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { usePanelView, useTaskData } from '../contexts/task-providers'
import { ChatView } from './chat-view/chat-view'
import { TaskHeader } from './task-header/task-header'
import { TaskDetailSkeleton } from './task-skeletons'

function TaskPageComponent() {
  const { taskId } = useParams({ from: '/tasks/$taskId' })

  // Use specialized hooks that only cause re-renders when specific data changes
  const { isLoading, error } = useTaskData()
  const { rightPanelView } = usePanelView()

  // All hooks at the top before any conditional logic
  const { data: serverStatus } = useServerStatus(taskId, {
    enabled: rightPanelView === 'preview',
  })

  if (error) {
    return <ErrorAlert error={error} />
  }

  if (isLoading) {
    return <TaskDetailSkeleton />
  }

  return (
    <div className="flex flex-col h-full w-full">
      <TaskHeader />
      <ResizablePanelGroup direction="horizontal" className="flex-1 w-full">
        {/* Left Panel - Chat */}
        <ResizablePanel
          defaultSize={40}
          minSize={25}
          maxSize={60}
          className="h-full"
          style={{
            minWidth: 'max(25%, 300px)',
            maxWidth: 'min(60%, 800px)',
          }}
        >
          <ErrorBoundary componentName="Chat View">
            <ChatView className="h-full" />
          </ErrorBoundary>
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Right Panel - Files/Preview */}
        <ResizablePanel className="flex flex-col">
          <div className="flex-1 overflow-hidden">
            <ErrorBoundary componentName={rightPanelView === 'files' ? 'File Explorer' : 'Preview'}>
              {rightPanelView === 'files' ? (
                <FileExplorer taskId={taskId} />
              ) : (
                <Preview config={{ url: serverStatus?.url || '' }} serverStatus={serverStatus} />
              )}
            </ErrorBoundary>
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}

export const TaskPage = React.memo(TaskPageComponent)
