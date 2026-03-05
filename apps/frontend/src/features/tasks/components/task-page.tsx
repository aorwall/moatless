import type { MessageContextInput } from '@moatless/api-client'
import React, { useCallback, useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { useTaskEventSubscription } from '@/features/events'
import { FileExplorer } from '@/features/files'
import { Preview, SandboxStatusOverlay } from '@/features/preview'
import { useHistoricalSandboxEvents } from '@/features/preview/hooks/use-task-events'
import type { ElementInfo, SandboxEvent, StartupEvent } from '@/features/preview/types'
import { useSandboxStatus } from '@/features/tasks'
import { useSandboxRetry } from '@/features/tasks/hooks/use-sandbox-status'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { FullPageError } from '@/lib/components/full-page-error'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { SidebarTrigger } from '@/lib/components/ui/sidebar'
import { Skeleton } from '@/lib/components/ui/skeleton'
import type { Event } from '@/lib/types/events'
import { useTaskId } from '../contexts/task-providers'
import { useTaskDetailQuery } from '../hooks/use-task-queries'
import { usePreviewPath, useTaskView } from '../hooks/use-task-view'
import { ChatContainer } from './chat-view/chat-container.tsx'
import { TaskLeftHeader } from './task-header/task-left-header'
import { TaskPageLayout } from './task-page-layout'
import { TaskDetailSkeleton } from './task-skeletons'
import { WorkspaceHeader } from './workspace-header'

/** Map a raw Event to a SandboxEvent for the Events tab. */
function toSandboxEvent(event: Event): SandboxEvent | null {
  if (event.type !== 'sandbox' && event.type !== 'server') {
    return null
  }
  const rawSubType = (event as { subType?: string }).subType
  const payload = (event as { payload?: Record<string, unknown> }).payload ?? {}
  return {
    eventId: event.eventId,
    type: event.type as 'sandbox' | 'server',
    subType: rawSubType ?? '',
    timestamp: event.timestamp ? new Date(event.timestamp).toISOString() : new Date().toISOString(),
    isError: event.isError,
    message: payload.message != null ? String(payload.message) : undefined,
    reason: payload.reason != null ? String(payload.reason) : undefined,
    serverName:
      event.type === 'server' && payload.serverName != null
        ? String(payload.serverName)
        : undefined,
    payload,
  }
}

/** Check if a sandbox event subType is a startup-phase event. */
function isStartupSubType(subType: string | undefined): boolean {
  return subType === 'initializing' || subType === 'k8s_event' || subType === 'sandbox_event'
}

/** Check if a sandbox event subType signals the end of the startup phase. */
function isStartupEndSubType(subType: string | undefined): boolean {
  return (
    subType === 'ready' || subType === 'error' || subType === 'stopped' || subType === 'removed'
  )
}

function TaskPageComponent() {
  const taskId = useTaskId()

  const taskQuery = useTaskDetailQuery(taskId)

  // State for managing context items
  const [contextItems, setContextItems] = useState<MessageContextInput[]>([])

  // Ref for command executor exposed by ChatContainer
  const commandExecutorRef = useRef<((commandName: string) => void) | null>(null)

  const handleCommandExecutorReady = useCallback((executor: (commandName: string) => void) => {
    commandExecutorRef.current = executor
  }, [])

  // K8s startup events for the initializing overlay
  const [startupEvents, setStartupEvents] = useState<StartupEvent[]>([])

  // Sandbox/server lifecycle events for Events tab and recent events box
  const [sandboxEvents, setSandboxEvents] = useState<SandboxEvent[]>([])

  // Track seen event IDs for deduplication between historical and live SSE events
  const seenEventIds = useRef<Set<string>>(new Set())

  // Fetch historical sandbox/server events so events persist across page reloads
  const historicalEventsQuery = useHistoricalSandboxEvents(taskId)

  // Seed sandboxEvents from historical data when query resolves
  useEffect(() => {
    const data = historicalEventsQuery.data
    if (!data || data.length === 0) {
      return
    }

    const mapped: SandboxEvent[] = []
    for (const event of data) {
      if (seenEventIds.current.has(event.eventId)) {
        continue
      }
      seenEventIds.current.add(event.eventId)
      const se = toSandboxEvent(event)
      if (se) {
        mapped.push(se)
      }
    }

    if (mapped.length > 0) {
      setSandboxEvents((prev) => {
        // Merge: historical first, then any live events that arrived before the query resolved
        const liveOnly = prev.filter((e) => !seenEventIds.current.has(e.eventId))
        return [...mapped, ...liveOnly]
      })
    }
  }, [historicalEventsQuery.data])

  const handleTaskEvent = useCallback((event: Event) => {
    // Append sandbox/server events for the Events tab (with dedup)
    const se = toSandboxEvent(event)
    if (se) {
      if (seenEventIds.current.has(event.eventId)) {
        return
      }
      seenEventIds.current.add(event.eventId)
      setSandboxEvents((prev) => [...prev, se])
    }

    if (event.type === 'sandbox') {
      const rawSubType = (event as { subType?: string }).subType
      if (isStartupSubType(rawSubType)) {
        const payload = (event as { payload?: Record<string, unknown> }).payload ?? {}
        if (payload.reason || payload.message) {
          setStartupEvents((prev) => [
            ...prev,
            {
              reason: String(payload.reason ?? ''),
              message: String(payload.message ?? ''),
              eventType: String(payload.eventType ?? 'Normal'),
              timestamp: event.timestamp
                ? new Date(event.timestamp).toISOString()
                : new Date().toISOString(),
            },
          ])
        }
      }
      if (isStartupEndSubType(rawSubType)) {
        setStartupEvents([])
      }
    }
  }, [])

  useTaskEventSubscription(taskId, handleTaskEvent)

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

  // Fetch sandbox status to determine capabilities and server URL
  // Fetch sandbox status only once the task is known to be open
  const sandboxStatusQuery = useSandboxStatus(taskId, taskQuery.data)
  const sandboxStatus = sandboxStatusQuery.data
  const servers = sandboxStatus?.servers

  // Hook for retrying/starting sandbox (used by files view)
  const retryMutation = useSandboxRetry(taskId)

  // Get current view from URL with smart defaults
  const { view, setView, isMobile } = useTaskView({
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
        const newContext: MessageContextInput = {
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
    return (
      <div className="flex flex-col h-full">
        <div className="h-[58px] px-4 py-2 border-b flex items-center">
          <SidebarTrigger />
        </div>
        <FullPageError error={taskQuery.error} onRetry={() => taskQuery.refetch()} />
      </div>
    )
  }

  if (taskQuery.isLoading) {
    return <TaskDetailSkeleton />
  }

  // Determine which workspace content to show based on unified view
  // Don't render final content until sandbox status is loaded to ensure correct view
  const workspaceContent = (() => {
    if (sandboxStatusQuery.isLoading) {
      return (
        <div className="p-4 space-y-4">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-64 w-full" />
          <Skeleton className="h-32 w-full" />
        </div>
      )
    }

    if (sandboxStatusQuery.error) {
      return <ErrorAlert error={sandboxStatusQuery.error} />
    }

    if (view === 'files') {
      const isSandboxReady = sandboxStatus?.environmentStatus === 'ready'
      return (
        <ErrorBoundary componentName="File Explorer">
          <div className="h-full flex flex-col">
            <WorkspaceHeader activeView={view} onViewChange={setView} servers={servers} />
            <div className="flex-1 min-h-0 relative">
              {isSandboxReady ? (
                <FileExplorer taskId={taskId} />
              ) : (
                sandboxStatus?.environmentStatus && (
                  <SandboxStatusOverlay
                    environmentStatus={sandboxStatus.environmentStatus}
                    environmentError={sandboxStatus.environmentError ?? undefined}
                    sandboxEvents={sandboxEvents}
                    onRetry={() => retryMutation.mutate()}
                    isRetrying={retryMutation.isPending}
                    retryError={retryMutation.error}
                  />
                )
              )}
            </div>
          </div>
        </ErrorBoundary>
      )
    }

    const activeServerName = view === 'chat' ? undefined : view

    // Render Preview for server views - it handles sandbox status internally
    return (
      <ErrorBoundary componentName="Preview">
        <Preview
          taskId={taskId}
          environmentStatus={sandboxStatus?.environmentStatus}
          environmentError={sandboxStatus?.environmentError ?? undefined}
          startupEvents={startupEvents}
          onToggleElementContext={handleToggleElementContext}
          onNavigationReady={handleNavigationReady}
          onTroubleshootRequest={handleTroubleshootRequest}
          servers={servers}
          activeServer={activeServerName}
          resources={sandboxStatus?.resources ?? undefined}
          sandboxEvents={sandboxEvents}
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
