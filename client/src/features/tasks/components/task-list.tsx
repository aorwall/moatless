import type { Task } from '@shared/types/task'
import { Link } from '@tanstack/react-router'
import { useEnvironmentsList } from '@/features/environments/hooks/use-environments'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'

interface TaskListProps {
  tasks: Array<Task>
  loading?: boolean
}

function getRelativeTime(date: Date | string): string {
  const now = new Date()
  const then = new Date(date)
  const seconds = Math.floor((now.getTime() - then.getTime()) / 1000)

  if (seconds < 60) return 'just now'
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 7) return `${days}d ago`
  const weeks = Math.floor(days / 7)
  if (weeks < 4) return `${weeks}w ago`
  const months = Math.floor(days / 30)
  if (months < 12) return `${months}mo ago`
  const years = Math.floor(days / 365)
  return `${years}y ago`
}

export function TaskList({ tasks, loading }: TaskListProps) {
  const { data: environments } = useEnvironmentsList()
  
  // Create a map of environment IDs to names
  const environmentMap =
    environments?.reduce(
      (acc, env) => {
        acc[env.id] = env.name
        return acc
      },
      {} as Record<string, string>,
    ) || {}
  
  // Sort tasks by last message time, newest first
  const sortedTasks = [...tasks].sort((a, b) => {
    return (
      new Date(b.lastMessageAt || b.createdAt).getTime() -
      new Date(a.lastMessageAt || a.createdAt).getTime()
    )
  })
  
  if (loading) {
    return <div className="text-center py-8 text-muted-foreground">Loading tasks...</div>
  }
  
  if (tasks.length === 0) {
    return <div className="text-center py-8 text-muted-foreground">No tasks found</div>
  }
  
  return (
    <TooltipProvider>
      <div className="rounded-md border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-[50%]">Task</TableHead>
              <TableHead>Environment</TableHead>
              <TableHead>Last Activity</TableHead>
              <TableHead className="text-right">Status</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {sortedTasks.map((task) => {
              const statusConfig = getTaskStatusConfig(task.status)
              const taskTitle =
                task.iterations?.[0]?.initialMessage?.split('\n')[0] || 
                `Task ${task.taskId.slice(0, 8)}`
              const environmentName = environmentMap[task.environmentId] || task.environmentId
              
              return (
                <TableRow key={task.taskId} className="cursor-pointer">
                  <TableCell className="font-medium max-w-0">
                    <Link 
                      to="/tasks/$taskId" 
                      params={{ taskId: task.taskId }}
                      className="block hover:text-primary truncate"
                    >
                      {taskTitle}
                    </Link>
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {environmentName}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {getRelativeTime(task.lastMessageAt || task.createdAt)}
                  </TableCell>
                  <TableCell className="text-right">
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <div className={`inline-flex items-center gap-2 ${statusConfig.color}`}>
                          <span className="text-sm">{statusConfig.tooltip}</span>
                          {statusConfig.icon}
                        </div>
                      </TooltipTrigger>
                      <TooltipContent>
                        <p className="text-xs">
                          {task.status === 'running' && 'Agent is currently running'}
                          {task.status === 'open' && 'Ready for new messages'}
                          {task.status === 'error' && 'Last iteration failed'}
                          {task.status === 'merged' && 'Changes have been merged'}
                          {task.status === 'closed' && 'Task has been closed'}
                        </p>
                      </TooltipContent>
                    </Tooltip>
                  </TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      </div>
    </TooltipProvider>
  )
}
