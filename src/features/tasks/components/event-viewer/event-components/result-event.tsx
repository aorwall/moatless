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
  usage: {
    input_tokens: number;
    cache_creation_input_tokens: number;
    cache_read_input_tokens: number;
    output_tokens: number;
  };
}

export function ResultEvent({ event }: ResultEventProps) {
  const payload = event.payload as ResultPayload;

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

  const totalTokens =
    payload.usage.input_tokens +
    payload.usage.cache_creation_input_tokens +
    payload.usage.cache_read_input_tokens +
    payload.usage.output_tokens;

  return (
    <EventHeader event={event}>
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          {payload.is_error ? (
            <AlertCircle className="h-4 w-4 text-red-500 dark:text-red-400" />
          ) : (
            <CheckCircle className="h-4 w-4 text-green-500 dark:text-green-400" />
          )}
          <span
            className={`text-sm font-medium ${
              payload.is_error
                ? 'text-red-700 dark:text-red-300'
                : 'text-green-700 dark:text-green-300'
            }`}
          >
            Result: {payload.subtype}
          </span>
        </div>

        <div
          className={`text-sm rounded-lg p-3 ${
            payload.is_error
              ? 'bg-red-50 dark:bg-red-900/20 text-red-800 dark:text-red-200'
              : 'bg-green-50 dark:bg-green-900/20 text-green-800 dark:text-green-200'
          }`}
        >
          <p className="font-medium mb-2">{payload.result}</p>

          <div className="grid grid-cols-2 gap-2 mt-3 text-xs">
            <div className="flex items-center gap-1">
              <Clock className="h-3 w-3" />
              <span>Duration: {formatDuration(payload.duration_ms)}</span>
            </div>
            <div className="flex items-center gap-1">
              <span>API Time: {formatDuration(payload.duration_api_ms)}</span>
            </div>
            <div className="flex items-center gap-1">
              <span>Turns: {payload.num_turns}</span>
            </div>
            <div className="flex items-center gap-1">
              <Coins className="h-3 w-3" />
              <span>Cost: ${payload.total_cost_usd.toFixed(4)}</span>
            </div>
          </div>
        </div>

        <div className="text-xs text-slate-500 dark:text-slate-400 bg-slate-100 dark:bg-slate-800 rounded p-2">
          <div className="font-mono">
            <div>Total tokens: {totalTokens.toLocaleString()}</div>
            <div className="mt-1 space-y-0.5 text-slate-400 dark:text-slate-500">
              <div>• Input: {payload.usage.input_tokens.toLocaleString()}</div>
              <div>
                • Cache creation:{' '}
                {payload.usage.cache_creation_input_tokens.toLocaleString()}
              </div>
              <div>
                • Cache read:{' '}
                {payload.usage.cache_read_input_tokens.toLocaleString()}
              </div>
              <div>
                • Output: {payload.usage.output_tokens.toLocaleString()}
              </div>
            </div>
          </div>
        </div>
      </div>
    </EventHeader>
  );
}
