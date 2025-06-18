import {
  CheckCircle,
  ChevronDown,
  ChevronRight,
  MessageSquare,
} from 'lucide-react';
import { useState } from 'react';

import { Button } from '@/lib/components/ui/button';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible';
import { Markdown } from '@/lib/components/ui/markdown';

import type { ParsedEvent } from '../hooks/use-claude-code-parser';
import { EventRenderer } from './event-renderer';

interface EventGroupProps {
  inputEvent: ParsedEvent;
  resultEvent: ParsedEvent;
  intermediateEvents: Array<ParsedEvent>;
}

const formatDuration = (durationMs: number): string => {
  if (durationMs < 1000) {
    return `${durationMs}ms`;
  }
  if (durationMs < 60000) {
    return `${(durationMs / 1000).toFixed(1)}s`;
  }
  const minutes = Math.floor(durationMs / 60000);
  const seconds = Math.floor((durationMs % 60000) / 1000);
  return `${minutes}m ${seconds}s`;
};

export function EventGroup({
  inputEvent,
  resultEvent,
  intermediateEvents,
}: EventGroupProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  // Extract input message from payload
  const inputPayload = inputEvent.data as Record<string, unknown>;
  const inputMessage = (inputPayload.message as string) || '';

  // Extract result from payload
  const resultPayload = resultEvent.data as Record<string, unknown>;
  const resultData = (resultPayload.result as string) || '';
  const isError = resultPayload.is_error as boolean;
  const duration = resultPayload.duration_ms as number;
  const usage = resultPayload.usage as Record<string, unknown>;

  return (
    <div className="border rounded-lg overflow-hidden shadow-sm">
      {/* Input Section */}
      <div className="bg-gray-50 dark:bg-gray-900/50 p-6">
        <div className="flex items-start gap-3">
          <MessageSquare className="h-5 w-5 text-gray-600 dark:text-gray-400 mt-0.5 flex-shrink-0" />
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 mb-3">
              <span className="text-sm font-semibold text-gray-900 dark:text-gray-100">
                Input
              </span>
              <span className="text-xs text-gray-500 dark:text-gray-400">
                {new Date(inputEvent.timestamp).toLocaleTimeString()}
              </span>
            </div>
            <Markdown
              content={inputMessage}
              className="text-sm text-gray-700 dark:text-gray-300"
            />
          </div>
        </div>
      </div>

      {/* Intermediate Events Section */}
      {intermediateEvents.length > 0 && (
        <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
          <CollapsibleTrigger asChild>
            <Button
              variant="ghost"
              className="w-full justify-between p-4 h-auto bg-gray-100 dark:bg-gray-800/50 hover:bg-gray-200 dark:hover:bg-gray-800 border-t border-b"
            >
              <span className="text-sm text-gray-600 dark:text-gray-400">
                {intermediateEvents.length} intermediate event
                {intermediateEvents.length !== 1 ? 's' : ''}
              </span>
              {isExpanded ? (
                <ChevronDown className="h-4 w-4 text-gray-500" />
              ) : (
                <ChevronRight className="h-4 w-4 text-gray-500" />
              )}
            </Button>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <div className="p-4 bg-gray-50 dark:bg-gray-900/20 border-t border-b border-gray-200 dark:border-gray-700">
              <div className="space-y-3">
                {intermediateEvents.map((event) => (
                  <div
                    key={event.event_id}
                    className="p-3 bg-white dark:bg-gray-800 rounded border border-gray-200 dark:border-gray-700"
                  >
                    <div className="flex items-center gap-2 mb-2">
                      <span className="text-xs font-medium text-gray-600 dark:text-gray-400 uppercase">
                        {event.type}
                      </span>
                      <span className="text-xs text-gray-500 dark:text-gray-500">
                        {new Date(event.timestamp).toLocaleTimeString()}
                      </span>
                    </div>
                    <EventRenderer event={event} isCompact={true} />
                  </div>
                ))}
              </div>
            </div>
          </CollapsibleContent>
        </Collapsible>
      )}

      {/* Result Section */}
      <div className="bg-white dark:bg-gray-800 p-6">
        <div className="flex items-start gap-3">
          <CheckCircle
            className={`h-5 w-5 mt-0.5 flex-shrink-0 ${
              isError
                ? 'text-red-500 dark:text-red-400'
                : 'text-green-500 dark:text-green-400'
            }`}
          />
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 mb-3">
              <span className="text-sm font-semibold text-gray-900 dark:text-gray-100">
                {isError ? 'Error' : 'Result'}
              </span>
              <span className="text-xs text-gray-500 dark:text-gray-400">
                {new Date(resultEvent.timestamp).toLocaleTimeString()}
              </span>
              {duration && (
                <span className="text-xs text-gray-500 dark:text-gray-400">
                  • {formatDuration(duration)}
                </span>
              )}
            </div>

            {/* Usage info */}
            {usage && (
              <div className="mb-3 flex flex-wrap gap-4 text-xs text-gray-500 dark:text-gray-400">
                {typeof usage.input_tokens === 'number' && (
                  <span>
                    Input: {usage.input_tokens.toLocaleString()} tokens
                  </span>
                )}
                {typeof usage.output_tokens === 'number' && (
                  <span>
                    Output: {usage.output_tokens.toLocaleString()} tokens
                  </span>
                )}
                {typeof usage.cache_read_input_tokens === 'number' && (
                  <span>
                    Cache Read: {usage.cache_read_input_tokens.toLocaleString()}{' '}
                    tokens
                  </span>
                )}
              </div>
            )}

            <Markdown
              content={resultData}
              className="text-sm text-gray-700 dark:text-gray-300"
            />
          </div>
        </div>
      </div>
    </div>
  );
}
