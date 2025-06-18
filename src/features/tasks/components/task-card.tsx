import { Link } from '@tanstack/react-router';
import { Clock, GitBranch, Minus, Plus } from 'lucide-react';

import { Badge } from '@/lib/components/ui/badge';
import { Card, CardContent } from '@/lib/components/ui/card';
import type { Task } from '@/types/task';

interface TaskCardProps {
  task: Task;
}

export function TaskCard({ task }: TaskCardProps) {
  return (
    <Link to="/tasks/$taskId" params={{ taskId: task.id }}>
      <Card
        className={`cursor-pointer transition-colors hover:bg-muted/50 ${
          task.status === 'error' ? 'opacity-75 hover:opacity-100' : ''
        }`}
      >
        <CardContent className="p-6">
          <div className="flex items-start justify-between space-x-4">
            <div className="space-y-2 flex-1">
              <h3 className="text-lg font-semibold leading-none tracking-tight">
                {task.title}
              </h3>
              <div className="flex items-center space-x-4 text-sm text-muted-foreground">
                <div className="flex items-center space-x-1">
                  <Clock className="h-4 w-4" />
                  <span>{task.time}</span>
                </div>
                <div className="flex items-center space-x-1">
                  <GitBranch className="h-4 w-4" />
                  <span>{task.project}</span>
                </div>
              </div>
            </div>
            <div className="flex items-center space-x-2">
              {task.changes && (
                <>
                  <Badge variant="outline" className="space-x-1">
                    <Plus className="h-3 w-3" />
                    <span>{task.changes.added}</span>
                  </Badge>
                  <Badge variant="outline" className="space-x-1">
                    <Minus className="h-3 w-3" />
                    <span>{task.changes.removed}</span>
                  </Badge>
                </>
              )}
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
        </CardContent>
      </Card>
    </Link>
  );
}
