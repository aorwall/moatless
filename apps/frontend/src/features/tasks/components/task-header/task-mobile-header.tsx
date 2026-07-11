import { ViewSwitcher } from '@/features/tasks/components/view-switcher'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useSandboxStatus } from '@/features/tasks/hooks/use-sandbox-status'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useTaskView } from '@/features/tasks/hooks/use-task-view'
import { SidebarTrigger } from '@/lib/components/ui/sidebar'

/**
 * Mobile task-page header. The mobile layout shows one panel at a time, so this
 * header carries the essentials for that experience: the sidebar trigger, the
 * task title, and the view switcher that swaps between chat and the workspace
 * views. Purpose-built for `mobile-task-layout` — the desktop chat-panel header
 * (`TaskLeftHeader`) stays focused on the desktop split.
 */
export function TaskMobileHeader() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: sandboxStatus } = useSandboxStatus(taskId, taskQuery.data)
  const servers = sandboxStatus?.servers
  const { view, setView } = useTaskView({ fallback: 'chat' })
  const task = taskQuery.data

  if (!task) {
    return null
  }

  const { name, description } = task
  const taskTitle = name || description?.split('\n')[0] || 'Untitled'

  return (
    <div className="absolute top-0 left-0 right-0 z-10 h-[58px] px-4 py-2 border-b flex items-center justify-between gap-2 flex-shrink-0 bg-background">
      <div className="flex items-center gap-3 min-w-0">
        <SidebarTrigger />
        <span className="font-medium text-sm truncate" title={taskTitle}>
          {taskTitle}
        </span>
      </div>

      <ViewSwitcher currentView={view} onViewChange={setView} servers={servers} />
    </div>
  )
}
