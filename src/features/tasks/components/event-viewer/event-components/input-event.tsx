import { User } from 'lucide-react';

import type { Event } from '@/types/event';

import { EventHeader } from './event-header';

interface InputEventProps {
  event: Event;
}

export function InputEvent({ event }: InputEventProps) {
  const payload = event.payload as {
    type: string;
    message: string | { type: string; text: string } | unknown;
  };

  // Extract the message text based on different possible structures
  let messageText = '';

  if (typeof payload.message === 'string') {
    messageText = payload.message;
  } else if (payload.message && typeof payload.message === 'object') {
    // Handle case where message is an object with text property
    const msgObj = payload.message as Record<string, unknown>;
    if ('text' in msgObj) {
      messageText = String(msgObj.text);
    } else if ('message' in msgObj) {
      messageText = String(msgObj.message);
    } else {
      // Fallback: stringify the object
      messageText = JSON.stringify(payload.message);
    }
  }

  return (
    <EventHeader event={event}>
      <div className="space-y-2">
        <div className="flex items-center gap-2">
          <User className="h-4 w-4 text-slate-500 dark:text-slate-400" />
          <span className="text-sm font-medium text-slate-700 dark:text-slate-200">
            {payload.type === 'user' ? 'User Input' : 'Input'}
          </span>
        </div>

        <div className="text-sm text-slate-600 dark:text-slate-300 bg-slate-50 dark:bg-slate-800 rounded-lg p-3">
          <p className="whitespace-pre-wrap break-words">{messageText}</p>
        </div>
      </div>
    </EventHeader>
  );
}
