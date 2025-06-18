import { Link } from '@tanstack/react-router';
import { ArrowLeft, RefreshCw, Share, StopCircle, Wifi } from 'lucide-react';
import { toast } from 'sonner';

import { useStopAgent } from '@/features/sessions/hooks/use-sessions';
import { Badge } from '@/lib/components/ui/badge';
import { Button } from '@/lib/components/ui/button';
import { Switch } from '@/lib/components/ui/switch';
import { cn } from '@/lib/utils';
import { getErrorMessage } from '@/lib/utils/error';
import type { SessionMetadata } from '@/types/session';

interface SessionHeaderProps {
  metadata: SessionMetadata;
  streamingEnabled: boolean;
  isConnected: boolean;
  onStreamingToggle: () => void;
}

export function SessionHeader({
  metadata,
  streamingEnabled,
  isConnected,
  onStreamingToggle,
}: SessionHeaderProps) {
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
    stopAgent.mutate(metadata.session_id);
  };

  return (
    <header className="border-b">
      <div className="flex h-16 items-center justify-between px-4">
        <div className="flex items-center space-x-4">
          <Link to="/">
            <Button variant="ghost" size="icon">
              <ArrowLeft className="h-4 w-4" />
            </Button>
          </Link>
          <div className="space-y-1">
            <h1 className="text-lg font-semibold">
              Session {metadata.session_id.slice(0, 8)}
            </h1>
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
              <Badge variant="outline">
                {metadata.total_messages} messages
              </Badge>
              <Badge variant="outline">
                ${metadata.total_cost_usd.toFixed(4)}
              </Badge>
            </div>
          </div>
        </div>
        <div className="flex items-center space-x-2">
          <div className="flex items-center space-x-2">
            <Wifi
              className={cn(
                'h-4 w-4',
                isConnected ? 'text-green-500' : 'text-muted-foreground',
              )}
            />
            <span className="text-sm text-muted-foreground">Live updates</span>
            <Switch
              checked={streamingEnabled}
              onCheckedChange={onStreamingToggle}
              disabled={metadata.status !== 'active'}
            />
          </div>
          {metadata.status === 'active' && (
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
          <Button variant="outline" size="sm">
            <RefreshCw className="mr-2 h-4 w-4" />
            Refresh
          </Button>
          <Button variant="outline" size="sm">
            <Share className="mr-2 h-4 w-4" />
            Share
          </Button>
        </div>
      </div>
    </header>
  );
}
