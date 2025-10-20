import type { MessageContext } from '@moatless/types'
import React, { useCallback, useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { FileExplorer } from '@/features/file-viewer'
import { Preview } from '@/features/preview'
import type { ElementInfo } from '@/features/preview/types'
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

  // State for managing context items
  const [contextItems, setContextItems] = useState<MessageContext[]>([])

  // Fetch workspace status to determine capabilities and server URL
  // Fetch workspace status only once the task is known to be open
  const workspaceStatusQuery = useWorkspaceStatus(taskId, taskQuery.data)
  const workspaceStatus = workspaceStatusQuery.data
  const serverManagement = workspaceStatus?.capabilities.serverManagement

  // Track if we've already applied the serverManagement-based default
  const hasAppliedServerDefault = useRef(false)

  // Handler to add element to context
  const handleAddElementToContext = useCallback(
    (element: ElementInfo) => {
      // Check for duplicates by componentId
      const isDuplicate = contextItems.some((item) => {
        if (item.type === 'element' && element.componentId) {
          return item.data.componentId === element.componentId
        }
        return false
      })

      if (isDuplicate) {
        toast.info('Element already added to context', {
          description: `${element.componentName || 'Element'} is already in the context`,
        })
        return
      }

      const newContext: MessageContext = {
        type: 'element',
        data: {
          componentId: element.componentId,
          path: element.path,
          line: element.line,
          column: element.column,
          componentName: element.componentName,
        },
      }
      setContextItems((prev) => [...prev, newContext])
      toast.success('Element added to context', {
        description: `${element.componentName || 'Element'} added successfully`,
      })
    },
    [contextItems],
  )

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
          <ChatContainer
            className="flex-1 min-h-0"
            contextItems={contextItems}
            onContextItemsChange={setContextItems}
          />
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
              <Preview taskId={taskId} onAddElementToContext={handleAddElementToContext} />
            )}
          </ErrorBoundary>
        )
      }
    />
  )
}

export const TaskPage = React.memo(TaskPageComponent)
