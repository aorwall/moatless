import { Clock, DollarSign, Hash, MessageSquare } from 'lucide-react';

import { Badge } from '@/lib/components/ui/badge';
import { ScrollArea } from '@/lib/components/ui/scroll-area';
import { Separator } from '@/lib/components/ui/separator';
import type { ClaudeMessage, SessionMetadata } from '@/types/session';

interface SessionSidebarProps {
  metadata: SessionMetadata;
  messages: Array<ClaudeMessage>;
}

export function SessionSidebar({ metadata, messages }: SessionSidebarProps) {
  const duration = metadata.last_message_at
    ? new Date(metadata.last_message_at).getTime() -
      new Date(metadata.created_at).getTime()
    : 0;
  const durationStr = `${Math.floor(duration / 60000)}m ${Math.floor((duration % 60000) / 1000)}s`;

  const userMessages = messages.filter((m) => m.type === 'user').length;
  const assistantMessages = messages.filter(
    (m) => m.type === 'assistant',
  ).length;
  const resultMessages = messages.filter((m) => m.type === 'result').length;

  return (
    <div className="w-80 border-r">
      <ScrollArea className="h-full">
        <div className="p-6 space-y-6">
          {/* Session Info */}
          <div className="space-y-4">
            <h3 className="font-semibold">Session Information</h3>

            <div className="space-y-3 text-sm">
              <div className="flex items-center space-x-2">
                <Hash className="h-4 w-4 text-muted-foreground" />
                <span className="text-muted-foreground">Session ID:</span>
                <span className="font-mono">
                  {metadata.session_id.slice(0, 8)}
                </span>
              </div>

              <div className="flex items-center space-x-2">
                <Clock className="h-4 w-4 text-muted-foreground" />
                <span className="text-muted-foreground">Duration:</span>
                <span>{durationStr}</span>
              </div>

              <div className="flex items-center space-x-2">
                <DollarSign className="h-4 w-4 text-muted-foreground" />
                <span className="text-muted-foreground">Cost:</span>
                <span>${metadata.total_cost_usd.toFixed(4)}</span>
              </div>

              <div className="flex items-center space-x-2">
                <MessageSquare className="h-4 w-4 text-muted-foreground" />
                <span className="text-muted-foreground">Messages:</span>
                <span>{metadata.total_messages}</span>
              </div>
            </div>
          </div>

          <Separator />

          {/* Message Breakdown */}
          <div className="space-y-3">
            <h3 className="font-semibold">Message Breakdown</h3>
            <div className="space-y-2">
              <div className="flex justify-between items-center">
                <span className="text-sm text-muted-foreground">
                  User Messages
                </span>
                <Badge variant="outline">{userMessages}</Badge>
              </div>
              <div className="flex justify-between items-center">
                <span className="text-sm text-muted-foreground">
                  Assistant Messages
                </span>
                <Badge variant="outline">{assistantMessages}</Badge>
              </div>
              <div className="flex justify-between items-center">
                <span className="text-sm text-muted-foreground">Results</span>
                <Badge variant="outline">{resultMessages}</Badge>
              </div>
            </div>
          </div>

          <Separator />

          {/* Agent Info */}
          <div className="space-y-3">
            <h3 className="font-semibold">Agent Details</h3>
            <div className="space-y-2">
              <div className="flex justify-between items-center">
                <span className="text-sm text-muted-foreground">Type</span>
                <Badge variant="secondary">{metadata.agent_type}</Badge>
              </div>
              <div className="flex justify-between items-center">
                <span className="text-sm text-muted-foreground">Status</span>
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
              </div>
            </div>
          </div>

          <Separator />

          {/* Timestamps */}
          <div className="space-y-3">
            <h3 className="font-semibold">Timestamps</h3>
            <div className="space-y-2 text-sm">
              <div>
                <span className="text-muted-foreground">Created:</span>
                <p className="font-mono text-xs mt-1">
                  {new Date(metadata.created_at).toLocaleString()}
                </p>
              </div>
              <div>
                <span className="text-muted-foreground">Last Activity:</span>
                <p className="font-mono text-xs mt-1">
                  {new Date(metadata.last_message_at).toLocaleString()}
                </p>
              </div>
            </div>
          </div>
        </div>
      </ScrollArea>
    </div>
  );
}
