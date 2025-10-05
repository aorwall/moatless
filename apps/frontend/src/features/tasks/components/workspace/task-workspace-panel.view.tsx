import type { ReactNode } from 'react'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { WorkspaceHeader } from '../workspace-header'

type WorkspaceView = 'files' | 'preview'

interface TaskWorkspacePanelViewProps {
  activeView: WorkspaceView
  onViewChange: (view: WorkspaceView) => void
  isLoading: boolean
  error: Error | null
  headerCenter?: ReactNode
  headerRight?: ReactNode
  filesPanel: ReactNode
  previewPanel: ReactNode
}

export function TaskWorkspacePanelView({
  activeView,
  onViewChange,
  isLoading,
  error,
  headerCenter,
  headerRight,
  filesPanel,
  previewPanel,
}: TaskWorkspacePanelViewProps) {
  if (isLoading) {
    return (
      <div className="p-4 space-y-4">
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-64 w-full" />
        <Skeleton className="h-32 w-full" />
      </div>
    )
  }

  if (error) {
    return <ErrorAlert error={error} />
  }

  const activeComponent = activeView === 'files' ? filesPanel : previewPanel
  const componentName = activeView === 'files' ? 'File Explorer' : 'Preview'

  return (
    <div className="flex h-full w-full flex-col overflow-hidden">
      <WorkspaceHeader
        activeView={activeView}
        onViewChange={onViewChange}
        centerContent={headerCenter}
        rightContent={headerRight}
      />
      <div className="flex-1 overflow-hidden">
        <ErrorBoundary componentName={componentName}>{activeComponent}</ErrorBoundary>
      </div>
    </div>
  )
}
