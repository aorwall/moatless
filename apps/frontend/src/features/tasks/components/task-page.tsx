import { useParams } from '@tanstack/react-router'
import React, { useCallback, useEffect, useState } from 'react'
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

  // Compute constraints for the left panel width in pixels
  const calculatePanelSizeConstraints = useCallback((windowWidth: number) => {
    const minWidthPx = 300
    const maxWidthPx = 600
    const defaultWidthPx = 600

    // Convert pixel values to percentages
    const minSizePercent = (minWidthPx / windowWidth) * 100
    const maxSizePercent = (maxWidthPx / windowWidth) * 100
    const defaultSizePercent = Math.min(
      Math.max((defaultWidthPx / windowWidth) * 100, minSizePercent),
      maxSizePercent,
    )

    return { minSizePercent, maxSizePercent, defaultSizePercent }
  }, [])

  const [panelConstraints, setPanelConstraints] = useState(() => {
    const windowWidth = typeof window !== 'undefined' ? window.innerWidth : 1200
    return calculatePanelSizeConstraints(windowWidth)
  })

  // Update constraints on window resize
  useEffect(() => {
    const handleResize = () => {
      const windowWidth = window.innerWidth
      setPanelConstraints(calculatePanelSizeConstraints(windowWidth))
    }

    window.addEventListener('resize', handleResize)
    return () => window.removeEventListener('resize', handleResize)
  }, [calculatePanelSizeConstraints])

  // Handle panel resize with pixel constraints
  const handleLeftPanelResize = useCallback(
    (size: number) => {
      const windowWidth = window.innerWidth
      const panelWidthPx = (windowWidth * size) / 100

      // Constrain to 300-600px range
      const constrainedWidthPx = Math.max(300, Math.min(600, panelWidthPx))
      const constrainedSizePercent = (constrainedWidthPx / windowWidth) * 100

      setPanelSizes({ left: constrainedSizePercent, right: 100 - constrainedSizePercent })
      localStorage.setItem('leftPanelSize', constrainedSizePercent.toString())
    },
    [setPanelSizes],
  )

  useEffect(() => {
    // Restore panel sizes from localStorage on mount only
    const restorePanelSizes = () => {
      const savedLeftSize = localStorage.getItem('leftPanelSize')
      if (savedLeftSize) {
        const windowWidth = window.innerWidth
        const savedSizePercent = Number(savedLeftSize)
        const savedWidthPx = (windowWidth * savedSizePercent) / 100

        // Ensure saved size is within constraints
        const constrainedWidthPx = Math.max(300, Math.min(600, savedWidthPx))
        const constrainedSizePercent = (constrainedWidthPx / windowWidth) * 100

        setPanelSizes({
          left: constrainedSizePercent,
          right: 100 - constrainedSizePercent,
        })
      }
    }

    restorePanelSizes()
  }, [setPanelSizes])

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
