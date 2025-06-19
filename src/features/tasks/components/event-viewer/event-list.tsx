import {
  AlertCircle,
  CheckCircle,
  ChevronRight,
  Clock,
  FileText,
  MessageSquare,
  Settings,
  User,
} from 'lucide-react';
import { useState } from 'react';

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible';

interface Event {
  event_id: string;
  type: string;
  sub_type?: string | null;
  task_id: string;
  timestamp: string;
  is_error: boolean;
  payload: unknown;
}

interface EventListProps {
  events: Array<Event>;
}

const getEventIcon = (type: string, isError: boolean) => {
  if (isError) {
    return <AlertCircle className="h-4 w-4 text-red-500 dark:text-red-400" />;
  }

  switch (type) {
    case 'environment':
    case 'workspace':
      return (
        <Settings className="h-4 w-4 text-slate-500 dark:text-slate-400" />
      );
    case 'input':
      return <User className="h-4 w-4 text-slate-500 dark:text-slate-400" />;
    case 'message':
      return (
        <MessageSquare className="h-4 w-4 text-slate-500 dark:text-slate-400" />
      );
    case 'tool_call':
      return (
        <Settings className="h-4 w-4 text-slate-500 dark:text-slate-400" />
      );
    case 'tool_result':
      return (
        <CheckCircle className="h-4 w-4 text-slate-500 dark:text-slate-400" />
      );
    case 'result':
      return (
        <CheckCircle className="h-4 w-4 text-slate-500 dark:text-slate-400" />
      );
    default:
      return (
        <FileText className="h-4 w-4 text-slate-500 dark:text-slate-400" />
      );
  }
};

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

const getEventDisplayText = (event: Event): string => {
  const payload = event.payload as Record<string, unknown>;

  // Handle message events
  if (event.type === 'message' && payload.message) {
    const message = payload.message as Record<string, unknown>;

    // Check for tool calls
    if (message.content && Array.isArray(message.content)) {
      const toolCall = message.content.find(
        (c: unknown) => (c as Record<string, unknown>).type === 'tool_use',
      );
      if (toolCall) {
        const tool = toolCall as Record<string, unknown>;
        const inputStr = tool.input
          ? JSON.stringify(tool.input).slice(0, 50)
          : '';
        return `Tool: ${tool.name}${inputStr ? ` - ${inputStr}...` : ''}`;
      }

      // Check for text content
      const textContent = message.content.find(
        (c: unknown) => (c as Record<string, unknown>).type === 'text',
      );
      if (textContent) {
        const text = textContent as Record<string, unknown>;
        if (text.text && typeof text.text === 'string') {
          return (
            text.text.slice(0, 100) + (text.text.length > 100 ? '...' : '')
          );
        }
      }
    }

    // Check for tool results
    if (
      message.role === 'user' &&
      message.content &&
      Array.isArray(message.content)
    ) {
      const toolResult = message.content.find(
        (c: unknown) => (c as Record<string, unknown>).type === 'tool_result',
      );
      if (toolResult) {
        const result = toolResult as Record<string, unknown>;
        const content = (result.content as string) || '';
        return `Result: ${content.slice(0, 100)}${
          content.length > 100 ? '...' : ''
        }`;
      }
    }
  }

  // Handle input events
  if (event.type === 'input' && payload.message) {
    const message = payload.message as string;
    const msg = message.slice(0, 100);
    return `${payload.type}: ${msg}${message.length > 100 ? '...' : ''}`;
  }

  // Handle workspace events
  if (event.type === 'workspace') {
    if (event.sub_type === 'setup_started') {
      return 'Workspace setup started';
    }
    if (event.sub_type === 'worktree_created') {
      return `Worktree created: ${payload.path || ''}`;
    }
    if (event.sub_type === 'setup_completed') {
      return 'Workspace setup completed';
    }
  }

  // Default: show sub_type or a snippet of payload
  if (event.sub_type) {
    return event.sub_type.replace(/_/g, ' ');
  }

  // Try to extract meaningful text from payload
  const payloadStr = JSON.stringify(payload);
  return payloadStr.slice(0, 100) + (payloadStr.length > 100 ? '...' : '');
};

const PayloadRenderer = ({ payload }: { payload: unknown }) => {
  return (
    <div className="bg-slate-50 dark:bg-slate-800 rounded-lg p-4 mt-3 w-full">
      <pre className="text-xs text-slate-600 dark:text-slate-300 overflow-x-auto whitespace-pre-wrap font-mono leading-relaxed break-words">
        {JSON.stringify(payload, null, 2)}
      </pre>
    </div>
  );
};

const EventItem = ({ event }: { event: Event }) => {
  const [isExpanded, setIsExpanded] = useState(false);
  const { time, date } = formatTimestamp(event.timestamp);

  return (
    <div
      className={`w-full border rounded-lg transition-all duration-200 ${
        event.is_error
          ? 'border-red-200 dark:border-red-800 bg-red-50/30 dark:bg-red-900/10'
          : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 hover:border-slate-300 dark:hover:border-slate-600 hover:shadow-sm'
      }`}
    >
      <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
        <CollapsibleTrigger asChild>
          <div className="p-4 cursor-pointer hover:bg-slate-50/50 dark:hover:bg-slate-800/50 transition-colors w-full">
            <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2 sm:gap-4 w-full">
              <div className="flex flex-col sm:flex-row sm:items-center gap-2 sm:gap-3 min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <div className="flex-shrink-0">
                    {getEventIcon(event.type, event.is_error)}
                  </div>
                  <div className="flex items-center flex-wrap gap-2">
                    <span className="text-sm font-medium text-slate-700 dark:text-slate-200">
                      {event.type}
                    </span>
                    {event.sub_type && (
                      <span className="text-xs text-slate-500 dark:text-slate-400 bg-slate-100 dark:bg-slate-800 px-2 py-0.5 rounded">
                        {event.sub_type}
                      </span>
                    )}
                    {event.is_error && (
                      <span className="text-xs text-red-600 dark:text-red-400 bg-red-100 dark:bg-red-900/30 px-2 py-0.5 rounded">
                        error
                      </span>
                    )}
                  </div>
                </div>

                <div className="text-sm text-slate-600 dark:text-slate-300 break-words sm:truncate min-w-0 flex-1">
                  {getEventDisplayText(event)}
                </div>
              </div>

              <div className="flex items-center justify-between sm:justify-end gap-3 flex-shrink-0 text-xs text-slate-500 dark:text-slate-400">
                <div className="flex items-center gap-1">
                  <Clock className="h-3 w-3" />
                  <span className="font-mono">{time}</span>
                  <span className="hidden sm:inline">{date}</span>
                </div>
                <ChevronRight
                  className={`h-4 w-4 transition-transform ${isExpanded ? 'rotate-90' : ''}`}
                />
              </div>
            </div>
          </div>
        </CollapsibleTrigger>

        <CollapsibleContent>
          <div className="px-4 pb-4 w-full">
            <div className="text-xs text-slate-400 dark:text-slate-500 font-mono mb-2 break-all">
              {event.event_id}
            </div>
            <PayloadRenderer payload={event.payload} />
          </div>
        </CollapsibleContent>
      </Collapsible>
    </div>
  );
};

export function EventList({ events }: EventListProps) {
  const sortedEvents = [...events].sort(
    (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime(),
  );

  return (
    <div className="space-y-2 w-full">
      {sortedEvents.map((event) => (
        <EventItem key={event.event_id} event={event} />
      ))}
    </div>
  );
}
