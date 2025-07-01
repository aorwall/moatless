import type { Task } from '@moatless/shared/types/task'
import { Link } from '@tanstack/react-router'
import { Calendar, Clock, DollarSign, MessageSquare, Server } from 'lucide-react'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'
import { Badge } from '@/lib/components/ui/badge'
import { Card, CardContent } from '@/lib/components/ui/card'

interface TaskCardProps {
  task: Task
}

export function TaskCard({ task }: TaskCardProps) {
  const createdDate = new Date(task.createdAt)
  const lastMessageDate = new Date(task.lastMessageAt)
  const statusConfig = getTaskStatusConfig(task.status)

  return (
    <Link to="/tasks/$taskId" params={{ taskId: task.taskId }}>
      <Card
        className={`cursor-pointer transition-colors hover:bg-muted/50 ${
          task.status === 'error' ? 'opacity-75 hover:opacity-100' : ''
        }`}
      >
        <CardContent className="p-6">
          <div className="space-y-4">
            {/* Header with title and status */}
            <div className="flex items-start justify-between space-x-4">
              <h3 className="text-lg font-semibold leading-none tracking-tight">{`Task ${task.taskId.slice(0, 8)}`}</h3>
              <div className="flex items-center space-x-2">
                <div className={`flex items-center ${statusConfig.color}`}>{statusConfig.icon}</div>
                <Badge
                  variant={
                    task.status === 'error'
                      ? 'destructive'
                      : task.status === 'open' || task.status === 'running'
                        ? 'default'
                        : 'secondary'
                  }
                >
                  {statusConfig.tooltip}
                </Badge>
              </div>
            </div>

            {/* Metadata grid */}
            <div className="grid grid-cols-2 gap-4 text-sm">
              <div className="space-y-2">
                <div className="flex items-center space-x-2 text-muted-foreground">
                  <Calendar className="h-4 w-4" />
                  <span>Created: {createdDate.toLocaleDateString()}</span>
                </div>
                <div className="flex items-center space-x-2 text-muted-foreground">
                  <Clock className="h-4 w-4" />
                  <span>Last activity: {lastMessageDate.toLocaleTimeString()}</span>
                </div>
                <div className="flex items-center space-x-2 text-muted-foreground">
                  <Server className="h-4 w-4" />
                  <span>Agent: {task.agentType}</span>
                </div>
              </div>
              <div className="space-y-2">
                <div className="flex items-center space-x-2 text-muted-foreground">
                  <MessageSquare className="h-4 w-4" />
                  <span>{task.numTurns} turns</span>
                </div>
                <div className="flex items-center space-x-2 text-muted-foreground">
                  <DollarSign className="h-4 w-4" />
                  <span>${task.totalCostUsd.toFixed(4)}</span>
                </div>
                <div className="flex items-center space-x-2 text-muted-foreground">
                  <span>Environment: {task.environmentId}</span>
                </div>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>
    </Link>
  )
}
