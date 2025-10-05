import React, { useEffect, useRef } from 'react'
import { FileExplorer } from '@/features/file-viewer'
import { Preview } from '@/features/preview'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { usePanelView, useTaskId } from '../contexts/task-providers'
import { useTaskDetailQuery } from '../hooks/use-task-queries'
import { useWorkspaceStatus } from '../hooks/use-workspace-status'
import { ChatContainer } from './chat-view/chat-container.tsx'
import { TaskLeftHeader } from './task-header/task-left-header'
import { TaskPageLayout } from './task-page-layout'
import { TaskDetailSkeleton } from './task-skeletons'

function TaskPageComponent() {
  const taskId = useTaskId()

  const taskQuery = useTaskDetailQuery(taskId)
  const { rightPanelView, setRightPanelView } = usePanelView()

  // Fetch workspace status to determine capabilities and server URL
  // Fetch workspace status only once the task is known to be open
  const workspaceStatusQuery = useWorkspaceStatus(taskId, taskQuery.data)
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

  if (taskQuery.error) {
    return <ErrorAlert error={taskQuery.error} />
  }

  if (taskQuery.isLoading) {
    return <TaskDetailSkeleton />
  }

  return (
    <TaskPageLayout
      header={<TaskLeftHeader />}
      chat={
        <ErrorBoundary componentName="Chat View">
          <ChatContainer className="flex-1 min-h-0" />
        </ErrorBoundary>
      }
      workspacePanel={
        workspaceStatusQuery.isLoading ? (
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
        )
      }
    />
  )
}

export const TaskPage = React.memo(TaskPageComponent)
