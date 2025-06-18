'use client';

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
  message?: string;
  timestamp: string;
  is_error: boolean;
  payload: unknown;
}

interface EventListProps {
  events: Array<Event>;
}

const getEventIcon = (type: string, isError: boolean) => {
  if (isError) {
    return <AlertCircle className="h-4 w-4 text-red-500" />;
  }

  switch (type) {
    case 'environment':
      return <Settings className="h-4 w-4 text-slate-500" />;
    case 'input':
      return <User className="h-4 w-4 text-slate-500" />;
    case 'message':
      return <MessageSquare className="h-4 w-4 text-slate-500" />;
    case 'tool_call':
      return <Settings className="h-4 w-4 text-slate-500" />;
    case 'tool_result':
      return <CheckCircle className="h-4 w-4 text-slate-500" />;
    case 'result':
      return <CheckCircle className="h-4 w-4 text-slate-500" />;
    default:
      return <FileText className="h-4 w-4 text-slate-500" />;
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

const PayloadRenderer = ({ payload }: { payload: unknown }) => {
  return (
    <div className="bg-slate-50 rounded-lg p-4 mt-3">
      <pre className="text-xs text-slate-600 overflow-x-auto whitespace-pre-wrap font-mono leading-relaxed">
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
      className={`border rounded-lg transition-all duration-200 ${
        event.is_error
          ? 'border-red-200 bg-red-50/30'
          : 'border-slate-200 bg-white hover:border-slate-300 hover:shadow-sm'
      }`}
    >
      <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
        <CollapsibleTrigger asChild>
          <div className="p-4 cursor-pointer hover:bg-slate-50/50 transition-colors">
            <div className="flex items-center justify-between gap-4">
              <div className="flex items-center gap-3 min-w-0 flex-1">
                <div className="flex-shrink-0">
                  {getEventIcon(event.type, event.is_error)}
                </div>

                <div className="flex items-center gap-2 flex-shrink-0">
                  <span className="text-sm font-medium text-slate-700">
                    {event.type}
                  </span>
                  {event.sub_type && (
                    <span className="text-xs text-slate-500 bg-slate-100 px-2 py-0.5 rounded">
                      {event.sub_type}
                    </span>
                  )}
                  {event.is_error && (
                    <span className="text-xs text-red-600 bg-red-100 px-2 py-0.5 rounded">
                      error
                    </span>
                  )}
                </div>

                <div className="text-sm text-slate-600 truncate min-w-0">
                  {event.message}
                </div>
              </div>

              <div className="flex items-center gap-3 flex-shrink-0 text-xs text-slate-500">
                <div className="flex items-center gap-1">
                  <Clock className="h-3 w-3" />
                  <span className="font-mono">{time}</span>
                  <span>{date}</span>
                </div>
                <ChevronRight
                  className={`h-4 w-4 transition-transform ${isExpanded ? 'rotate-90' : ''}`}
                />
              </div>
            </div>
          </div>
        </CollapsibleTrigger>

        <CollapsibleContent>
          <div className="px-4 pb-4">
            <div className="text-xs text-slate-400 font-mono mb-2">
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
    (a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime(),
  );

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold text-slate-900">Event Log</h2>
        <span className="text-sm text-slate-500">{events.length} events</span>
      </div>

      <div className="space-y-2">
        {sortedEvents.map((event) => (
          <EventItem key={event.event_id} event={event} />
        ))}
      </div>
    </div>
  );
}
