import { Loader2, StopCircle, WifiOff } from 'lucide-react';
import { toast } from 'sonner';

import { useStopAgent } from '@/features/tasks/hooks/use-tasks';
import { Badge } from '@/lib/components/ui/badge';
import { Button } from '@/lib/components/ui/button';
import { SidebarTrigger } from '@/lib/components/ui/sidebar.tsx';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip';
import { getErrorMessage } from '@/lib/utils/error';
import type { TaskMetadata } from '@/types/task';

interface TaskHeaderProps {
  metadata: TaskMetadata;
  streamingError?: string | null;
}

export function TaskHeader({ metadata, streamingError }: TaskHeaderProps) {
  const stopAgent = useStopAgent({
    onSuccess: () => {
      toast.success('Agent stopped successfully');
    },
    onError: (error) => {
      console.error('Failed to stop agent:', error);
      toast.error('Failed to stop agent', {
        description: getErrorMessage(error),
      });
    },
  });

  const handleStop = () => {
    stopAgent.mutate(metadata.task_id);
  };

  return (
    <header className="border-b">
      <div className="flex h-16 items-center justify-between px-4">
        <div className="flex items-center space-x-4">
          <SidebarTrigger />

          <div className="space-y-1">
            <div className="flex items-center space-x-2">
              <h1 className="text-lg font-semibold">
                {metadata.title || `Task ${metadata.task_id.slice(0, 8)}`}
              </h1>
              {metadata.is_running && (
                <Loader2 className="h-4 w-4 animate-spin text-blue-500" />
              )}
            </div>
            <div className="flex items-center space-x-2 text-sm text-muted-foreground">
              <span>{new Date(metadata.created_at).toLocaleString()}</span>
              <span>{metadata.agent_type}</span>
              <Badge
                variant={
                  metadata.status === 'active'
                    ? 'default'
                    : metadata.status === 'error'
                      ? 'destructive'
                      : 'secondary'
                }
              >
                {metadata.status}
              </Badge>
              <Badge variant="outline">{metadata.num_turns} turns</Badge>
              <Badge variant="outline">
                ${metadata.total_cost_usd.toFixed(4)}
              </Badge>
            </div>
          </div>
        </div>
        <div className="flex items-center space-x-2">
          <div className="flex items-center space-x-2">
            {streamingError && (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <div className="flex items-center space-x-1">
                      <WifiOff className="h-4 w-4 text-red-500" />
                      <span className="text-sm text-red-500">
                        Streaming error
                      </span>
                    </div>
                  </TooltipTrigger>
                  <TooltipContent>
                    <p>{streamingError}</p>
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            )}
          </div>
          {metadata.is_running && (
            <Button
              variant="outline"
              size="sm"
              onClick={handleStop}
              disabled={stopAgent.isPending}
            >
              <StopCircle className="mr-2 h-4 w-4" />
              {stopAgent.isPending ? 'Stopping...' : 'Stop'}
            </Button>
          )}
        </div>
      </div>
    </header>
  );
}
