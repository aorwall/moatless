import { Link } from '@tanstack/react-router';
import {
  Calendar,
  Clock,
  DollarSign,
  Loader2,
  MessageSquare,
  Server,
} from 'lucide-react';

import { Badge } from '@/lib/components/ui/badge';
import { Card, CardContent } from '@/lib/components/ui/card';
import type { Task } from '@/types/task';

interface TaskCardProps {
  task: Task;
}

const getWorkspaceStatusBadge = (status: Task['workspaceStatus']) => {
  switch (status) {
    case 'not_setup':
      return (
        <Badge variant="outline" className="text-xs">
          Not Set Up
        </Badge>
      );
    case 'setting_up':
      return (
        <Badge variant="outline" className="text-xs flex items-center gap-1">
          <Loader2 className="h-3 w-3 animate-spin" />
          Setting Up
        </Badge>
      );
    case 'idle':
      return (
        <Badge variant="outline" className="text-xs">
          Idle
        </Badge>
      );
    case 'running':
      return (
        <Badge variant="outline" className="text-xs flex items-center gap-1">
          <Loader2 className="h-3 w-3 animate-spin" />
          Running
        </Badge>
      );
    default:
      return (
        <Badge variant="outline" className="text-xs">
          Unknown
        </Badge>
      );
  }
};

export function TaskCard({ task }: TaskCardProps) {
  const createdDate = new Date(task.createdAt);
  const lastMessageDate = new Date(task.lastMessageAt);

  return (
    <Link to="/tasks/$taskId" params={{ taskId: task.id }}>
      <Card
        className={`cursor-pointer transition-colors hover:bg-muted/50 ${
          task.status === 'error' ? 'opacity-75 hover:opacity-100' : ''
        }`}
      >
        <CardContent className="p-6">
          <div className="space-y-4">
            {/* Header with title and status */}
            <div className="flex items-start justify-between space-x-4">
              <h3 className="text-lg font-semibold leading-none tracking-tight">
                {task.title}
              </h3>
              <div className="flex items-center space-x-2">
                {task.status === 'active' && (
                  <Badge variant="default">Active</Badge>
                )}
                {task.status === 'completed' && (
                  <Badge variant="secondary">Completed</Badge>
                )}
                {task.status === 'error' && (
                  <Badge variant="destructive">Error</Badge>
                )}
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
                  <span>
                    Last activity: {lastMessageDate.toLocaleTimeString()}
                  </span>
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
                  <span>Environment: {task.environmentName}</span>
                </div>
              </div>
            </div>

            {/* Workspace status */}
            <div className="flex items-center justify-between pt-2 border-t">
              <span className="text-sm text-muted-foreground">
                Workspace Status
              </span>
              {getWorkspaceStatusBadge(task.workspaceStatus)}
            </div>
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}
