import { MessageSquare, Wrench } from 'lucide-react';

import type { Event } from '@/types/event';

import { EventHeader } from './event-header';

interface MessageEventProps {
  event: Event;
}

const renderMessageContent = (content: unknown): React.ReactNode => {
  if (!content) {
    return null;
  }

  // Handle array of content items
  if (Array.isArray(content)) {
    return (
      <>
        {content.map((item, index) => {
          // Generate a more stable key based on content type and position
          const itemType =
            item && typeof item === 'object' && 'type' in item
              ? item.type
              : 'unknown';
          const key = `content-${itemType}-${index}`;
          return <div key={key}>{renderSingleContent(item)}</div>;
        })}
      </>
    );
  }

  // Handle single content item
  return renderSingleContent(content);
};

const renderSingleContent = (content: unknown): React.ReactNode => {
  if (!content || typeof content !== 'object') {
    return <span>{String(content)}</span>;
  }

  const contentObj = content as Record<string, unknown>;

  switch (contentObj.type) {
    case 'text':
      return (
        <p className="whitespace-pre-wrap break-words">
          {String(contentObj.text || '')}
        </p>
      );

    case 'tool_use':
      return (
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <Wrench className="h-3.5 w-3.5 text-blue-500" />
            <span className="font-medium text-blue-600 dark:text-blue-400">
              {String(contentObj.name || 'Tool')}
            </span>
          </div>
          {contentObj.input && (
            <div className="text-xs bg-slate-100 dark:bg-slate-700 rounded p-2 overflow-x-auto">
              <pre className="font-mono">
                {JSON.stringify(contentObj.input, null, 2)}
              </pre>
            </div>
          )}
        </div>
      );

    case 'tool_result':
      return (
        <div className="space-y-2">
          {contentObj.tool_use_id && (
            <div className="text-xs text-slate-500 dark:text-slate-400">
              Tool ID: {String(contentObj.tool_use_id)}
            </div>
          )}
          <div className="text-sm bg-green-50 dark:bg-green-900/20 text-green-800 dark:text-green-200 rounded p-2">
            {String(contentObj.content || 'Tool result')}
          </div>
        </div>
      );

    default:
      // Fallback for unknown content types
      return (
        <pre className="text-xs whitespace-pre-wrap break-words font-mono">
          {JSON.stringify(content, null, 2)}
        </pre>
      );
  }
};

export function MessageEvent({ event }: MessageEventProps) {
  const payload = event.payload as Record<string, unknown>;

  // Extract message object - it can have various structures
  const message = payload.message as Record<string, unknown> | null;
  if (!message) {
    return <div className="text-sm text-slate-500">No message content</div>;
  }

  const isToolCall = event.sub_type === 'tool_call';
  const messageType = payload.type || message.role || 'unknown';

  // Extract content based on message structure
  let content: unknown = null;

  if ('content' in message) {
    content = message.content;
  } else if ('text' in message) {
    content = { type: 'text', text: message.text };
  } else if ('message' in message) {
    content = { type: 'text', text: message.message };
  }

  return (
    <EventHeader event={event}>
      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <MessageSquare className="h-4 w-4 text-slate-500 dark:text-slate-400" />
            <span className="text-sm font-medium text-slate-700 dark:text-slate-200">
              {messageType === 'assistant'
                ? 'Assistant'
                : messageType === 'user'
                  ? 'User'
                  : messageType}
              {isToolCall && ' (Tool Call)'}
            </span>
          </div>
          {message.usage && typeof message.usage === 'object' && (
            <div className="text-xs text-slate-500 dark:text-slate-400 font-mono">
              {Number(
                (message.usage as Record<string, unknown>).input_tokens || 0,
              ).toLocaleString()}{' '}
              tokens
            </div>
          )}
        </div>

        <div className="text-sm text-slate-600 dark:text-slate-300 bg-slate-50 dark:bg-slate-800 rounded-lg p-3">
          {renderMessageContent(content) || (
            <span className="text-slate-400">No content</span>
          )}
        </div>

        {message.model && (
          <div className="text-xs text-slate-500 dark:text-slate-400">
            Model: {String(message.model)}
          </div>
        )}
      </div>
    </EventHeader>
  );
}
