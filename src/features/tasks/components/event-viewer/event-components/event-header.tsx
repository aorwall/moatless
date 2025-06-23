import { Clock } from 'lucide-react';

import { Progress } from '@/lib/components/ui/progress';
import type { Event } from '@/types/event';

interface EventHeaderProps {
  event: Event;
  children: React.ReactNode;
}

const formatTimestamp = (timestamp: string) => {
  const date = new Date(timestamp);
  return {
    time: date.toLocaleTimeString('en-US', {
      hour12: false,
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    }),
    date: date.toLocaleDateString('en-US', {
      month: 'short',
      day: 'numeric',
    }),
  };
};

export function EventHeader({ event, children }: EventHeaderProps) {
  const { time, date } = formatTimestamp(event.timestamp);

  return (
    <div className="flex items-start justify-between gap-4">
      <div className="flex-1">{children}</div>

      <div className="flex items-center gap-3 text-xs text-muted-foreground flex-shrink-0">
        {event.context_tokens > 0 && (
          <div className="flex items-center gap-2">
            <Progress
              value={(event.context_tokens / 200000) * 100}
              className="w-16 h-1.5"
            />
            <span className="font-mono text-xs">
              {event.context_tokens.toLocaleString()}
            </span>
          </div>
        )}
        <div className="flex items-center gap-1">
          <Clock className="h-3 w-3" />
          <span className="font-mono">{time}</span>
          <span className="hidden sm:inline">{date}</span>
        </div>
      </div>
    </div>
  );
}
