import { useParams } from '@tanstack/react-router'
import React from 'react'
import { FileExplorer } from '@/features/file-viewer'
import { Preview, useServerStatus } from '@/features/preview'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { usePanelView, useTaskData } from '../contexts/task-providers'
import { ChatView } from './chat-view/chat-view'
import { TaskLeftHeader } from './task-header/task-left-header'
import { TaskRightHeader } from './task-header/task-right-header'
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
    <div className="h-full w-full">
      <ResizablePanelGroup direction="horizontal" className="h-full w-full">
        {/* Left Panel - Chat with Header */}
        <ResizablePanel defaultSize={40} minSize={25} maxSize={60} className="flex flex-col">
          <TaskLeftHeader />
          <ErrorBoundary componentName="Chat View">
            <ChatView className="flex-1 min-h-0" />
          </ErrorBoundary>
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Right Panel - Files/Preview with Header */}
        <ResizablePanel className="flex flex-col">
          <TaskRightHeader />
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
