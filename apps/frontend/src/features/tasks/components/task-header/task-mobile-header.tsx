import { ViewSwitcher } from '@/features/tasks/components/view-switcher'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useSandboxStatus } from '@/features/tasks/hooks/use-sandbox-status'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useTaskView } from '@/features/tasks/hooks/use-task-view'
import { SidebarTrigger } from '@/lib/components/ui/sidebar'

/**
 * Mobile task-page navbar. A fixed top bar carrying the sidebar trigger (left)
 * and the chat/files/… view switcher (center). View-specific controls (the file
 * mode toggle, etc.) live in each panel's own top bar rather than in a header
 * dropdown, so the right slot is just a spacer that keeps the switcher centered.
 */
export function TaskMobileHeader() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: sandboxStatus } = useSandboxStatus(taskId, taskQuery.data)
  const servers = sandboxStatus?.servers
  const { view, setView } = useTaskView({ fallback: 'chat' })

  if (!taskQuery.data) {
    return null
  }

  return (
    <div className="flex h-[50px] flex-shrink-0 items-center gap-2 border-b bg-background px-3">
      {/* Left — sidebar trigger. flex-1 balances the right slot so the switcher
          stays centered regardless of the actions menu's presence. */}
      <div className="flex flex-1 items-center justify-start">
        <SidebarTrigger />
      </div>

      <ViewSwitcher currentView={view} onViewChange={setView} servers={servers} />

      {/* Right — spacer balancing the left slot so the switcher stays centered. */}
      <div className="flex flex-1 items-center justify-end" />
    </div>
  )
}
