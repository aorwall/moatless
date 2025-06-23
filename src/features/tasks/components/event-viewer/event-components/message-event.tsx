import { Wrench } from 'lucide-react';
import type { ReactNode } from 'react';

import type { Event } from '@/types/event';

import {
  BashToolCall,
  EditToolCall,
  GlobToolCall,
  GrepToolCall,
  ReadToolCall,
  TodoToolCall,
  WriteToolCall,
} from './tool-calls';

interface MessageEventProps {
  event: Event;
}

const renderMessageContent = (content: unknown): ReactNode => {
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

const renderSingleContent = (content: unknown): ReactNode => {
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

    case 'tool_use': {
      const toolName = String(contentObj.name || 'Tool');

      // Special handling for Bash tool calls
      if (
        toolName === 'Bash' &&
        contentObj.input &&
        typeof contentObj.input === 'object'
      ) {
        return (
          <BashToolCall
            input={
              contentObj.input as {
                command: string;
                description?: string;
                timeout?: number;
              }
            }
            name={toolName}
          />
        );
      }

      // Special handling for Edit tool calls
      if (
        toolName === 'Edit' &&
        contentObj.input &&
        typeof contentObj.input === 'object'
      ) {
        return (
          <EditToolCall
            input={
              contentObj.input as {
                file_path: string;
                old_string: string;
                new_string: string;
                replace_all?: boolean;
              }
            }
            name={toolName}
          />
        );
      }

      // Special handling for Read tool calls
      if (
        toolName === 'Read' &&
        contentObj.input &&
        typeof contentObj.input === 'object'
      ) {
        return (
          <ReadToolCall
            input={
              contentObj.input as {
                file_path: string;
                offset?: number;
                limit?: number;
              }
            }
            name={toolName}
          />
        );
      }

      // Special handling for TodoWrite tool calls
      if (
        toolName === 'TodoWrite' &&
        contentObj.input &&
        typeof contentObj.input === 'object'
      ) {
        return (
          <TodoToolCall
            input={
              contentObj.input as {
                todos: Array<{
                  id: string;
                  content: string;
                  status: 'pending' | 'in_progress' | 'completed';
                  priority: 'high' | 'medium' | 'low';
                }>;
              }
            }
            name={toolName}
          />
        );
      }

      // Special handling for Grep tool calls
      if (
        toolName === 'Grep' &&
        contentObj.input &&
        typeof contentObj.input === 'object'
      ) {
        return (
          <GrepToolCall
            input={
              contentObj.input as {
                pattern: string;
                path?: string;
                include?: string;
              }
            }
            name={toolName}
          />
        );
      }

      // Special handling for Glob tool calls
      if (
        toolName === 'Glob' &&
        contentObj.input &&
        typeof contentObj.input === 'object'
      ) {
        return (
          <GlobToolCall
            input={
              contentObj.input as {
                pattern: string;
                path?: string;
              }
            }
            name={toolName}
          />
        );
      }

      // Special handling for Write tool calls
      if (
        toolName === 'Write' &&
        contentObj.input &&
        typeof contentObj.input === 'object'
      ) {
        return (
          <WriteToolCall
            input={
              contentObj.input as {
                file_path: string;
                content: string;
              }
            }
            name={toolName}
          />
        );
      }

      // Default handling for other tool calls
      return (
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <Wrench className="h-3.5 w-3.5 text-muted-foreground" />
            <span className="font-medium text-foreground">{toolName}</span>
          </div>
          {contentObj.input ? (
            <div className="text-xs bg-muted rounded p-2 overflow-x-auto">
              <pre className="font-mono">
                {typeof contentObj.input === 'object'
                  ? JSON.stringify(contentObj.input, null, 2)
                  : String(contentObj.input)}
              </pre>
            </div>
          ) : null}
        </div>
      );
    }

    case 'tool_result':
      return (
        <div className="space-y-2">
          {contentObj.tool_use_id ? (
            <div className="text-xs text-muted-foreground">
              Tool ID: {String(contentObj.tool_use_id)}
            </div>
          ) : null}
          <div className="text-sm bg-muted text-muted-foreground rounded p-2">
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
    return (
      <div className="text-sm text-muted-foreground">No message content</div>
    );
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
    <div className="space-y-2">
      <div className="text-sm text-muted-foreground bg-muted rounded-lg p-3">
        {content ? (
          renderMessageContent(content)
        ) : (
          <span className="text-muted-foreground">No content</span>
        )}
      </div>
    </div>
  );
}
