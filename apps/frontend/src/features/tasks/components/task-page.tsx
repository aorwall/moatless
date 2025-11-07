import type { MessageContext } from '@moatless/types'
import React, { useCallback, useState } from 'react'
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

  // State for managing context items
  const [contextItems, setContextItems] = useState<MessageContext[]>([])

  // Fetch workspace status to determine capabilities and server URL
  // Fetch workspace status only once the task is known to be open
  const workspaceStatusQuery = useWorkspaceStatus(taskId, taskQuery.data)
  const workspaceStatus = workspaceStatusQuery.data
  const serverManagement = workspaceStatus?.capabilities.serverManagement

  // Derive panel view from user preference and capabilities
  const { rightPanelView } = usePanelView(serverManagement)

  // Handler to toggle element in context (add if not present, remove if present)
  const handleToggleElementContext = useCallback(
    (element: ElementInfo) => {
      // Check if element already exists in context by componentId
      const existingIndex = contextItems.findIndex((item) => {
        if (item.type === 'element' && element.componentId) {
          return item.data.componentId === element.componentId
        }
        return false
      })

      if (existingIndex !== -1) {
        // Element exists - remove it
        setContextItems((prev) => prev.filter((_, index) => index !== existingIndex))
        toast.info('Element removed from context', {
          description: `${element.componentName || 'Element'} removed from context`,
        })
      } else {
        // Element doesn't exist - add it
        const newContext: MessageContext = {
          type: 'element',
          referenceId: element.componentId,
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
          description: `${element.componentName || 'Element'} added to context`,
        })
      }
    },
    [contextItems],
  )

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
          <ErrorBoundary componentName={rightPanelView === 'files' ? 'File Explorer' : 'Preview'}>
            {rightPanelView === 'files' ? (
              <FileExplorer taskId={taskId} serverManagement={serverManagement} />
            ) : (
              <Preview
                taskId={taskId}
                serverManagement={serverManagement}
                onToggleElementContext={handleToggleElementContext}
              />
            )}
          </ErrorBoundary>
        )
      }
    />
  )
}

export const TaskPage = React.memo(TaskPageComponent)
