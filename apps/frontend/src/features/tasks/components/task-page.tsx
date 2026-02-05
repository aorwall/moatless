import type { MessageContext } from '@moatless/types'
import React, { useCallback, useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { Preview } from '@/features/preview'
import type { ElementInfo } from '@/features/preview/types'
import { useWorkspaceStatus } from '@/features/tasks'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { useTaskId } from '../contexts/task-providers'
import { useTaskDetailQuery } from '../hooks/use-task-queries'
import { usePreviewPath, useTaskView } from '../hooks/use-task-view'
import { ChatContainer } from './chat-view/chat-container.tsx'
import { TaskLeftHeader } from './task-header/task-left-header'
import { TaskPageLayout } from './task-page-layout'
import { TaskDetailSkeleton } from './task-skeletons'

function TaskPageComponent() {
  const taskId = useTaskId()

  const taskQuery = useTaskDetailQuery(taskId)

  // State for managing context items
  const [contextItems, setContextItems] = useState<MessageContext[]>([])

  // Ref for command executor exposed by ChatContainer
  const commandExecutorRef = useRef<((commandName: string) => void) | null>(null)

  const handleCommandExecutorReady = useCallback((executor: (commandName: string) => void) => {
    commandExecutorRef.current = executor
  }, [])

  // Ref for capturing preview navigation function (using ref to avoid re-render issues)
  const previewNavigateRef = useRef<((path: string) => void) | null>(null)

  // Stable callback that uses the ref
  const handlePreviewUrlClick = useCallback((path: string) => {
    if (previewNavigateRef.current) {
      previewNavigateRef.current(path)
    }
  }, [])

  // Callback to receive the navigation function from Preview
  const handleNavigationReady = useCallback((navigate: (path: string) => void) => {
    previewNavigateRef.current = navigate
  }, [])

  // Callback for troubleshoot button from proxy error pages — executes built-in command
  const handleTroubleshootRequest = useCallback((_reason: string) => {
    commandExecutorRef.current?.('troubleshoot-preview')
  }, [])

  // Get current preview path from URL state
  const { previewPath } = usePreviewPath()

  // Fetch workspace status to determine capabilities and server URL
  // Fetch workspace status only once the task is known to be open
  const workspaceStatusQuery = useWorkspaceStatus(taskId, taskQuery.data)
  const workspaceStatus = workspaceStatusQuery.data
  const servers = workspaceStatus?.servers

  // Get current view from URL with smart defaults
  const { view, isMobile } = useTaskView({
    servers,
  })

  // Sync preview path to context items whenever it changes
  useEffect(() => {
    if (!previewPath || previewPath === '/') {
      // Remove any existing preview URL context when preview is at root
      setContextItems((prev) => prev.filter((item) => item.type !== 'previewUrl'))
      return
    }

    // Add or update preview URL context
    setContextItems((prev) => {
      const existingIndex = prev.findIndex((item) => item.type === 'previewUrl')

      if (existingIndex !== -1) {
        // Update existing preview URL context
        return prev.map((item, idx) =>
          idx === existingIndex
            ? {
                type: 'previewUrl',
                referenceId: previewPath,
                data: { path: previewPath },
              }
            : item,
        )
      }

      // Add new preview URL context
      return [
        ...prev,
        {
          type: 'previewUrl',
          referenceId: previewPath,
          data: { path: previewPath },
        },
      ]
    })
  }, [previewPath])

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

  // Determine which workspace content to show based on unified view
  // Don't render final content until workspace status is loaded to ensure correct view
  const workspaceContent = (() => {
    if (workspaceStatusQuery.isLoading) {
      return (
        <div className="p-4 space-y-4">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-64 w-full" />
          <Skeleton className="h-32 w-full" />
        </div>
      )
    }

    if (workspaceStatusQuery.error) {
      return <ErrorAlert error={workspaceStatusQuery.error} />
    }

    const activeServerName = view === 'chat' ? undefined : view

    // Always render Preview - it handles sandbox status internally
    return (
      <ErrorBoundary componentName="Preview">
        <Preview
          taskId={taskId}
          environmentStatus={workspaceStatus?.environmentStatus}
          environmentError={workspaceStatus?.environmentError}
          onToggleElementContext={handleToggleElementContext}
          onNavigationReady={handleNavigationReady}
          onTroubleshootRequest={handleTroubleshootRequest}
          servers={servers}
          activeServer={activeServerName}
        />
      </ErrorBoundary>
    )
  })()

  return (
    <TaskPageLayout
      header={<TaskLeftHeader />}
      chat={
        <ErrorBoundary componentName="Chat View">
          <ChatContainer
            className="flex-1 min-h-0"
            contextItems={contextItems}
            onContextItemsChange={setContextItems}
            onPreviewUrlClick={handlePreviewUrlClick}
            onCommandExecutorReady={handleCommandExecutorReady}
          />
        </ErrorBoundary>
      }
      workspacePanel={workspaceContent}
      isMobile={isMobile}
      mobileView={view}
    />
  )
}

export const TaskPage = React.memo(TaskPageComponent)
