import { AlertCircle, CheckCircle, Clock, Coins } from 'lucide-react';

import type { Event } from '@/types/event';

import { EventHeader } from './event-header';

interface ResultEventProps {
  event: Event;
}

interface ResultPayload {
  type: string;
  subtype: string;
  is_error: boolean;
  duration_ms: number;
  duration_api_ms: number;
  num_turns: number;
  result: string;
  session_id: string;
  total_cost_usd: number;
  usage?: {
    input_tokens?: number;
    cache_creation_input_tokens?: number;
    cache_read_input_tokens?: number;
    output_tokens?: number;
  };
}

export function ResultEvent({ event }: ResultEventProps) {
  const payload = event.payload as unknown as ResultPayload;

  const formatDuration = (ms: number) => {
    if (ms < 1000) {
      return `${ms}ms`;
    }
    const seconds = Math.floor(ms / 1000);
    if (seconds < 60) {
      return `${seconds}s`;
    }
    const minutes = Math.floor(seconds / 60);
    const remainingSeconds = seconds % 60;
    return `${minutes}m ${remainingSeconds}s`;
  };

  return (
    <EventHeader event={event}>
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          {payload.is_error ? (
            <AlertCircle className="h-4 w-4 text-muted-foreground" />
          ) : (
            <CheckCircle className="h-4 w-4 text-muted-foreground" />
          )}
          <span
            className={`text-sm font-medium ${
              payload.is_error ? 'text-foreground' : 'text-muted-foreground'
            }`}
          >
            Result: {payload.subtype || 'Unknown'}
          </span>
        </div>

        <div
          className={`text-sm rounded-lg p-3 ${
            payload.is_error
              ? 'bg-muted text-foreground'
              : 'bg-secondary text-muted-foreground'
          }`}
        >
          <p className="font-medium mb-2">{payload.result || 'No result'}</p>

          <div className="grid grid-cols-2 gap-2 mt-3 text-xs">
            <div className="flex items-center gap-1">
              <Clock className="h-3 w-3" />
              <span>Duration: {formatDuration(payload.duration_ms || 0)}</span>
            </div>
            <div className="flex items-center gap-1">
              <span>
                API Time: {formatDuration(payload.duration_api_ms || 0)}
              </span>
            </div>
            <div className="flex items-center gap-1">
              <span>Turns: {payload.num_turns || 0}</span>
            </div>
            <div className="flex items-center gap-1">
              <Coins className="h-3 w-3" />
              <span>Cost: ${(payload.total_cost_usd || 0).toFixed(4)}</span>
            </div>
          </div>
        </div>
      </div>
    </EventHeader>
  );
}
