import { Activity, AlertCircle } from 'lucide-react';

import { Markdown } from '@/lib/components/ui/markdown';

import type {
  ParsedEvent,
  ParsedMessage,
} from '../../hooks/use-claude-code-parser';
import { MessageContent } from './message-content';
import { ToolCalls } from './tool-calls';
import { ToolResults } from './tool-results';

interface EventRendererProps {
  event: ParsedEvent;
  isCompact?: boolean;
}

const formatPayload = (payload: Record<string, unknown>): string => {
  try {
    return JSON.stringify(payload, null, 2);
  } catch {
    return String(payload);
  }
};

const renderMessageContent = (message: ParsedMessage) => {
  return (
    <div className="space-y-4">
      {/* Message content */}
      {message.content && <MessageContent content={message.content} />}

      {/* Tool calls */}
      {message.toolCalls && message.toolCalls.length > 0 && (
        <ToolCalls toolCalls={message.toolCalls} />
      )}

      {/* Tool results */}
      {message.toolResults && message.toolResults.length > 0 && (
        <ToolResults toolResults={message.toolResults} />
      )}
    </div>
  );
};

export function EventRenderer({
  event,
  isCompact = false,
}: EventRendererProps) {
  // Handle Claude message events
  if (
    event.type === 'message' &&
    typeof event.data === 'object' &&
    'role' in event.data
  ) {
    return renderMessageContent(event.data as ParsedMessage);
  }

  // For non-message events, show appropriate content
  if (event.type === 'start') {
    return (
      <div className="flex items-center gap-2 text-sm">
        <Activity className="h-4 w-4 text-green-500" />
        <span>Task started</span>
      </div>
    );
  }

  if (event.type === 'stop') {
    return (
      <div className="flex items-center gap-2 text-sm">
        <Activity className="h-4 w-4 text-yellow-500" />
        <span>Task stopped</span>
      </div>
    );
  }

  if (event.type === 'error') {
    return (
      <div className="space-y-2">
        <div className="flex items-center gap-2 text-sm">
          <AlertCircle className="h-4 w-4 text-red-500" />
          <span className="text-red-700 dark:text-red-300">Error occurred</span>
        </div>
        <pre className="text-sm font-mono whitespace-pre-wrap bg-red-50 dark:bg-red-900/20 p-2 rounded overflow-auto max-h-96 text-red-900 dark:text-red-100">
          {formatPayload(event.data as Record<string, unknown>)}
        </pre>
      </div>
    );
  }

  // Handle environment events
  if (event.type === 'environment') {
    const payload = event.data as Record<string, unknown>;
    const subtype = payload.subtype as string;

    // Extract payload data without task_id and environment_id
    const {
      task_id: _taskId,
      environment_id: _environmentId,
      ...displayData
    } = payload;

    return (
      <div className="space-y-2">
        <div className="flex items-center gap-2 text-sm">
          <Activity className="h-4 w-4 text-blue-500" />
          <span className="font-medium text-gray-700 dark:text-gray-300">
            Environment: {subtype}
          </span>
        </div>
        {Object.keys(displayData).length > 1 && (
          <div className="text-sm text-gray-600 dark:text-gray-400 space-y-1">
            {Object.entries(displayData).map(([key, value]) => {
              if (key === 'subtype') {
                return null;
              }
              return (
                <div key={key}>
                  <span className="font-medium">{key}:</span> {String(value)}
                </div>
              );
            })}
          </div>
        )}
      </div>
    );
  }

  // Handle input events (these should normally be grouped, but show them if standalone)
  if (event.type === 'input') {
    const payload = event.data as Record<string, unknown>;
    const message = (payload.message as string) || JSON.stringify(payload);
    return (
      <div className="space-y-2">
        <div className="flex items-center gap-2 text-sm font-medium text-gray-700 dark:text-gray-300">
          Input
        </div>
        <Markdown content={message} className="text-sm" />
      </div>
    );
  }

  // Handle result events (these should normally be grouped, but show them if standalone)
  if (event.type === 'result') {
    const payload = event.data as Record<string, unknown>;
    const result = (payload.result as string) || JSON.stringify(payload);
    const isError = payload.is_error as boolean;
    return (
      <div className="space-y-2">
        <div className="flex items-center gap-2 text-sm font-medium text-gray-700 dark:text-gray-300">
          {isError ? 'Error Result' : 'Result'}
        </div>
        <Markdown content={result} className="text-sm" />
      </div>
    );
  }

  // Default payload rendering for other event types
  return (
    <pre
      className={`font-mono whitespace-pre-wrap bg-muted p-2 rounded overflow-auto ${
        isCompact ? 'text-xs max-h-48' : 'text-sm max-h-96'
      }`}
    >
      {formatPayload(event.data as Record<string, unknown>)}
    </pre>
  );
}
