import type { GitSyncStatus } from '@moatless/types/types/api'
import type { Task } from '@moatless/types/types/task'
import { Link } from '@tanstack/react-router'
import { Trash2 } from 'lucide-react'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useDeleteTask } from '@/features/tasks/hooks/use-tasks'
import {
  formatGitSyncDetails,
  getGitSyncStatusConfig,
} from '@/features/tasks/utils/git-sync-status'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'
import { Button } from '@/lib/components/ui/button'
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

// Component for individual task row with git sync status
function TaskRow({
  task,
  repositoryName,
  onDelete,
  isDeleting = false,
}: {
  task: Task
  repositoryName: string
  onDelete: (taskId: string, event: React.MouseEvent) => void
  isDeleting?: boolean
}) {
  const statusConfig = getTaskStatusConfig(task.status)
  const taskTitle =
    task.iterations?.[0]?.initialMessage?.split('\n')[0] || `Task ${task.taskId.slice(0, 8)}`

  // Get git sync status from the last iteration
  const lastIteration = task.iterations?.[task.iterations.length - 1]
  const gitStatus = task.status === 'open' ? lastIteration?.gitStatus : null
  const gitSyncConfig = gitStatus ? getGitSyncStatusConfig(gitStatus.status as GitSyncStatus) : null

  return (
    <TableRow className="cursor-pointer">
      <TableCell className="font-medium max-w-0">
        <Link
          to="/tasks/$taskId"
          params={{ taskId: task.taskId }}
          className="block hover:text-primary"
          title={taskTitle}
        >
          <span className="line-clamp-1">{taskTitle}</span>
        </Link>
      </TableCell>
      <TableCell className="text-muted-foreground">{repositoryName}</TableCell>
      <TableCell className="text-muted-foreground">
        {getRelativeTime(task.lastMessageAt || task.createdAt)}
      </TableCell>
      <TableCell>
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
      <TableCell className="w-10">
        {task.status === 'open' && gitSyncConfig ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <div className={`inline-flex items-center justify-center ${gitSyncConfig.color}`}>
                {gitSyncConfig.icon}
              </div>
            </TooltipTrigger>
            <TooltipContent>
              <div>
                <p className="font-medium">{gitSyncConfig.tooltip}</p>
                {gitStatus && (
                  <p className="text-xs text-muted-foreground mt-1">
                    {formatGitSyncDetails(gitStatus)}
                  </p>
                )}
              </div>
            </TooltipContent>
          </Tooltip>
        ) : (
          <span className="text-xs text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell className="text-right">
        <Button
          variant="ghost"
          size="icon"
          onClick={(e) => onDelete(task.taskId, e)}
          disabled={isDeleting}
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      </TableCell>
    </TableRow>
  )
}

export function TaskList({ tasks, loading }: TaskListProps) {
  const { data: repositories } = useRepositoriesList()
  const deleteTask = useDeleteTask()

  // Create a map of repository IDs to names
  const repositoryMap =
    repositories?.reduce(
      (acc, repo) => {
        acc[repo.id] = repo.name
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

  const handleDelete = (taskId: string, event: React.MouseEvent) => {
    event.preventDefault()
    event.stopPropagation()
    if (confirm('Are you sure you want to delete this task?')) {
      deleteTask.mutate(taskId)
    }
  }

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
              <TableHead className="w-[45%]">Task</TableHead>
              <TableHead>Environment</TableHead>
              <TableHead>Last Activity</TableHead>
              <TableHead>Status</TableHead>
              <TableHead className="w-10">
                <Tooltip>
                  <TooltipTrigger>
                    <span className="sr-only">Git Sync Status</span>
                    <span aria-hidden="true">Git</span>
                  </TooltipTrigger>
                  <TooltipContent>Git Sync Status</TooltipContent>
                </Tooltip>
              </TableHead>
              <TableHead className="text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {sortedTasks.map((task) => {
              const repositoryName = repositoryMap[task.repositoryId] || task.repositoryId
              return (
                <TaskRow
                  key={task.taskId}
                  task={task}
                  repositoryName={repositoryName}
                  onDelete={handleDelete}
                  isDeleting={deleteTask.isPending}
                />
              )
            })}
          </TableBody>
        </Table>
      </div>
    </TooltipProvider>
  )
}
