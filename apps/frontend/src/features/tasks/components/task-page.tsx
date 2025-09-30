import { useParams } from '@tanstack/react-router'
import React, { useEffect, useRef } from 'react'
import { FileExplorer } from '@/features/file-viewer'
import { Preview } from '@/features/preview'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { usePanelView, useTaskData } from '../contexts/task-providers'
import { useWorkspaceStatusWithTask } from '../hooks/use-workspace-status'
import { ChatContainer } from './chat-view/chat-container.tsx'
import { TaskLeftHeader } from './task-header/task-left-header'
import { TaskDetailSkeleton } from './task-skeletons'

function TaskPageComponent() {
  const { taskId } = useParams({ from: '/tasks/$taskId' })

  // Use specialized hooks that only cause re-renders when specific data changes
  const { isLoading, error } = useTaskData()
  const { rightPanelView, setRightPanelView } = usePanelView()

  // Fetch workspace status to determine capabilities and server URL
  const workspaceStatusQuery = useWorkspaceStatusWithTask(taskId)
  const workspaceStatus = workspaceStatusQuery.data
  const serverManagement = workspaceStatus?.capabilities.serverManagement

  // Track if we've already applied the serverManagement-based default
  const hasAppliedServerDefault = useRef(false)

  // Apply initial default: show preview for tasks with serverManagement capability
  // This effect runs once when serverManagement data loads, before user interaction
  // Subsequent user toggles are respected (effect won't run again due to ref guard)
  useEffect(() => {
    if (
      !hasAppliedServerDefault.current &&
      serverManagement && // workspace has preview capability
      rightPanelView === 'files' // still on default files view (user hasn't toggled yet)
    ) {
      setRightPanelView('preview')
      hasAppliedServerDefault.current = true
    }
  }, [serverManagement, rightPanelView, setRightPanelView])

  const effectivePanelView = rightPanelView

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
            <ChatContainer className="flex-1 min-h-0" />
          </ErrorBoundary>
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Right Panel - Files/Preview */}
        <ResizablePanel className="flex flex-col">
          {workspaceStatusQuery.isLoading ? (
            <div className="p-4 space-y-4">
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-64 w-full" />
              <Skeleton className="h-32 w-full" />
            </div>
          ) : workspaceStatusQuery.error ? (
            <ErrorAlert error={workspaceStatusQuery.error} />
          ) : (
            <ErrorBoundary
              componentName={effectivePanelView === 'files' ? 'File Explorer' : 'Preview'}
            >
              {effectivePanelView === 'files' ? (
                <FileExplorer taskId={taskId} />
              ) : (
                <Preview taskId={taskId} />
              )}
            </ErrorBoundary>
          )}
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}

export const TaskPage = React.memo(TaskPageComponent)
