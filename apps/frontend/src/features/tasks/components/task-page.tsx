import { useParams } from '@tanstack/react-router'
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { FileExplorer } from '@/features/file-viewer'
import { Preview, useServerStatus } from '@/features/preview'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { usePanelSizes, usePanelView, useTaskData } from '../contexts/task-providers'
import { ChatView } from './chat-view/chat-view'
import { TaskHeader } from './task-header/task-header'
import { TaskDetailSkeleton } from './task-skeletons'

function TaskPageComponent() {
  const { taskId } = useParams({ from: '/tasks/$taskId' })

  // Use specialized hooks that only cause re-renders when specific data changes
  const { isLoading, error } = useTaskData()
  const { rightPanelView } = usePanelView()
  const { setPanelSizes } = usePanelSizes()

  // All hooks at the top before any conditional logic
  const { data: serverStatus } = useServerStatus(taskId, {
    enabled: rightPanelView === 'preview',
  })

  // Track window width with a single state update
  const [windowWidth, setWindowWidth] = useState(() =>
    typeof window !== 'undefined' ? window.innerWidth : 1200,
  )

  // Derive panel constraints from window width - no separate state needed
  const panelConstraints = useMemo(() => {
    const minWidthPx = 300
    const maxWidthPx = 1200
    const defaultWidthPx = 600

    // Convert pixel values to percentages
    const minSizePercent = (minWidthPx / windowWidth) * 100
    const maxSizePercent = (maxWidthPx / windowWidth) * 100
    const defaultSizePercent = Math.min(
      Math.max((defaultWidthPx / windowWidth) * 100, minSizePercent),
      maxSizePercent,
    )

    return { minSizePercent, maxSizePercent, defaultSizePercent }
  }, [windowWidth])

  // Debounce window resize updates
  const resizeTimeoutRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)

  useEffect(() => {
    const handleResize = () => {
      if (resizeTimeoutRef.current) {
        clearTimeout(resizeTimeoutRef.current)
      }
      resizeTimeoutRef.current = setTimeout(() => {
        setWindowWidth(window.innerWidth)
      }, 150) // Debounce by 150ms
    }

    window.addEventListener('resize', handleResize)
    return () => {
      window.removeEventListener('resize', handleResize)
      if (resizeTimeoutRef.current) {
        clearTimeout(resizeTimeoutRef.current)
      }
    }
  }, [])

  // Handle panel resize with pixel constraints
  const handleLeftPanelResize = useCallback(
    (size: number) => {
      const panelWidthPx = (windowWidth * size) / 100

      // Constrain to 300-800px range
      const constrainedWidthPx = Math.max(300, Math.min(800, panelWidthPx))
      const constrainedSizePercent = (constrainedWidthPx / windowWidth) * 100

      setPanelSizes({ left: constrainedSizePercent, right: 100 - constrainedSizePercent })
    },
    [windowWidth, setPanelSizes],
  )

  // Set initial panel sizes on mount
  useEffect(() => {
    setPanelSizes({
      left: panelConstraints.defaultSizePercent,
      right: 100 - panelConstraints.defaultSizePercent,
    })
  }, [panelConstraints.defaultSizePercent, setPanelSizes])

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
          defaultSize={panelConstraints.defaultSizePercent}
          minSize={panelConstraints.minSizePercent}
          maxSize={panelConstraints.maxSizePercent}
          onResize={handleLeftPanelResize}
          className="h-full"
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
