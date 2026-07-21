import { FullPageError } from '@/lib/components/full-page-error'
import { SidebarTrigger } from '@/lib/components/ui/sidebar'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { TaskDetailSkeleton } from '../components/task-skeletons'
import { useTaskId } from '../contexts/task-providers'
import { useTaskDetailQuery } from '../hooks/use-task-queries'
import { DesktopTaskLayout } from './desktop-task-layout'
import { KioskTaskLayout } from './kiosk-task-layout'
import { MobileTaskLayout } from './mobile-task-layout'

interface TaskPageProps {
  /**
   * Render a chrome-less, on-display variant of the task page: no task header,
   * no preview header (address bar, server tabs, inspector), no console, no
   * agent edit/plan mode selector. Tool calls collapse into a single counter
   * row per assistant message. Used by the `/tasks/$task-id/kiosk` route.
   */
  kiosk?: boolean
}

/**
 * Task page entry point: the single place that resolves the device/kiosk
 * decision, then delegates to one self-contained layout composition. Owns the
 * task-level loading/error gating; everything below is a plain composition of
 * shared panels — no `isMobile`/`kiosk` branching past this point.
 */
function TaskPageComponent({ kiosk = false }: TaskPageProps) {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const isMobile = useIsMobile()

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

  if (kiosk) {
    return <KioskTaskLayout />
  }

  return isMobile ? <MobileTaskLayout /> : <DesktopTaskLayout />
}

export const TaskPage = TaskPageComponent
