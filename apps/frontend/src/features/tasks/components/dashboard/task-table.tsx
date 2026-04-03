import type { TaskResponse } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { type ColumnDef, flexRender, getCoreRowModel, useReactTable } from '@tanstack/react-table'
import { ChevronLeft, ChevronRight, X } from 'lucide-react'
import { toast } from 'sonner'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useSandboxStatus } from '@/features/tasks/hooks/use-sandbox-status'
import { useCloseTask, useDeleteTask } from '@/features/tasks/hooks/use-tasks'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import { useApiError } from '@/lib/hooks/use-api-error'

const channelLabels: Record<string, string> = {
  slack: 'Slack',
  email: 'Email',
  webhook: 'Webhook',
  cron: 'Cron',
  telegram: 'Telegram',
  github_pr: 'GitHub PR',
}

function formatCost(cost?: number): string {
  if (cost === undefined || cost === null || cost === 0) {
    return '-'
  }
  return `$${cost.toFixed(2)}`
}

function formatDate(date: Date | string): string {
  const d = new Date(date)
  return d.toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

function TaskStatusCell({ task }: { task: TaskResponse }) {
  const { data: sandboxStatus } = useSandboxStatus(task.taskId, task)
  return <TaskStatusIcon task={task} sandbox={sandboxStatus} showText={true} />
}

function TaskActionCell({ task }: { task: TaskResponse }) {
  const { formatErrorForDisplay } = useApiError()
  const isClosed = task.status === 'closed'

  const closeTaskMutation = useCloseTask(task.taskId, {
    onError: (error) => {
      try {
        const formattedError = formatErrorForDisplay(error)
        toast.error(formattedError.title, { description: formattedError.message })
      } catch {
        toast.error('Failed to close task', { description: error.message })
      }
    },
  })

  const deleteTaskMutation = useDeleteTask({
    onError: (error) => {
      try {
        const formattedError = formatErrorForDisplay(error)
        toast.error(formattedError.title, { description: formattedError.message })
      } catch {
        toast.error('Failed to delete task', { description: error.message })
      }
    },
  })

  const handleClick = () => {
    if (isClosed) {
      deleteTaskMutation.mutate(task.taskId)
    } else {
      closeTaskMutation.mutate()
    }
  }

  return (
    <Button
      variant="ghost"
      size="icon"
      className="h-6 w-6 opacity-0 group-hover/row:opacity-100 transition-opacity"
      onClick={(e) => {
        e.stopPropagation()
        handleClick()
      }}
      disabled={closeTaskMutation.isPending || deleteTaskMutation.isPending}
    >
      <X className="h-4 w-4" />
      <span className="sr-only">{isClosed ? 'Delete task' : 'Close task'}</span>
    </Button>
  )
}

const columns: ColumnDef<TaskResponse>[] = [
  {
    id: 'status',
    header: 'Status',
    size: 120,
    cell: ({ row }) => <TaskStatusCell task={row.original} />,
  },
  {
    id: 'title',
    header: 'Name',
    cell: ({ row }) => {
      const task = row.original
      const title = task.name || task.description?.split('\n')[0] || 'Untitled'
      return (
        <Link
          to="/tasks/$task-id"
          params={{ 'task-id': task.taskId }}
          className="font-medium hover:underline line-clamp-1 max-w-[300px]"
        >
          {title}
        </Link>
      )
    },
  },
  {
    id: 'repository',
    header: 'Repository',
    cell: ({ row }) => (
      <span className="text-muted-foreground text-sm truncate max-w-[150px] block">
        {row.original.repositoryName || row.original.repositoryId}
      </span>
    ),
  },
  {
    id: 'tags',
    header: 'Tags',
    cell: ({ row }) => {
      const taskTags = row.original.tags
      if (!taskTags || taskTags.length === 0) {
        return null
      }
      const displayed = taskTags.slice(0, 3)
      const overflow = taskTags.length - 3
      return (
        <div className="flex gap-1 flex-wrap">
          {displayed.map((tag) => (
            <Badge key={tag.tagId} variant="secondary" className="text-xs">
              {tag.name}
            </Badge>
          ))}
          {overflow > 0 && (
            <Badge variant="outline" className="text-xs">
              +{overflow}
            </Badge>
          )}
        </div>
      )
    },
  },
  {
    id: 'channel',
    header: 'Channel',
    cell: ({ row }) => {
      const channels = row.original.channels
      if (!channels || channels.length === 0) {
        return null
      }
      const first = channels[0]
      const label = first.sourceName
        ? `${channelLabels[first.provider] ?? first.provider}: ${first.sourceName}`
        : (channelLabels[first.provider] ?? first.provider)
      return (
        <div className="flex gap-1 items-center">
          <Badge variant="secondary" className="text-xs">
            {label}
          </Badge>
          {channels.length > 1 && (
            <Badge variant="outline" className="text-xs">
              +{channels.length - 1}
            </Badge>
          )}
        </div>
      )
    },
  },
  {
    id: 'createdBy',
    header: 'Created By',
    cell: ({ row }) => (
      <span className="text-muted-foreground text-sm">
        {row.original.createdByDisplayName || row.original.createdBy || '-'}
      </span>
    ),
  },
  {
    id: 'createdAt',
    header: 'Created',
    cell: ({ row }) => (
      <span className="text-muted-foreground text-sm">{formatDate(row.original.createdAt)}</span>
    ),
  },
  {
    id: 'cost',
    header: 'Cost',
    cell: ({ row }) => (
      <span className="text-muted-foreground text-sm tabular-nums">
        {formatCost(row.original.totalCostUsd)}
      </span>
    ),
  },
  {
    id: 'actions',
    header: '',
    size: 48,
    cell: ({ row }) => <TaskActionCell task={row.original} />,
  },
]

interface TaskTableProps {
  tasks: TaskResponse[]
  total: number
  page: number
  pageSize: number
  onPageChange: (page: number) => void
}

export function TaskTable({ tasks, total, page, pageSize, onPageChange }: TaskTableProps) {
  const table = useReactTable({
    data: tasks,
    columns,
    getCoreRowModel: getCoreRowModel(),
  })

  const totalPages = Math.max(1, Math.ceil(total / pageSize))
  const startItem = (page - 1) * pageSize + 1
  const endItem = Math.min(page * pageSize, total)

  return (
    <div>
      <div className="rounded-md border">
        <Table>
          <TableHeader>
            {table.getHeaderGroups().map((headerGroup) => (
              <TableRow key={headerGroup.id}>
                {headerGroup.headers.map((header) => (
                  <TableHead key={header.id}>
                    {header.isPlaceholder
                      ? null
                      : flexRender(header.column.columnDef.header, header.getContext())}
                  </TableHead>
                ))}
              </TableRow>
            ))}
          </TableHeader>
          <TableBody>
            {table.getRowModel().rows.length > 0 ? (
              table.getRowModel().rows.map((row) => (
                <TableRow key={row.id} className="cursor-pointer group/row">
                  {row.getVisibleCells().map((cell) => (
                    <TableCell key={cell.id}>
                      {flexRender(cell.column.columnDef.cell, cell.getContext())}
                    </TableCell>
                  ))}
                </TableRow>
              ))
            ) : (
              <TableRow>
                <TableCell colSpan={columns.length} className="h-24 text-center">
                  No tasks found.
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </div>

      {/* Pagination */}
      {total > 0 && (
        <div className="flex items-center justify-between mt-4">
          <span className="text-sm text-muted-foreground">
            {startItem}-{endItem} of {total}
          </span>
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => onPageChange(page - 1)}
              disabled={page <= 1}
            >
              <ChevronLeft className="h-4 w-4" />
              Previous
            </Button>
            <span className="text-sm text-muted-foreground">
              Page {page} of {totalPages}
            </span>
            <Button
              variant="outline"
              size="sm"
              onClick={() => onPageChange(page + 1)}
              disabled={page >= totalPages}
            >
              Next
              <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}
