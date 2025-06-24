import { User } from 'lucide-react'

import type { Event } from '@/types/event'

interface InputEventProps {
  event: Event
}

export function InputEvent({ event }: InputEventProps) {
  const payload = event.payload as {
    type: string
    message: string | { type: string; text: string } | unknown
  }

  // Extract the message text based on different possible structures
  let messageText = ''

  if (typeof payload.message === 'string') {
    messageText = payload.message
  } else if (payload.message && typeof payload.message === 'object') {
    // Handle case where message is an object with text property
    const msgObj = payload.message as Record<string, unknown>
    if ('text' in msgObj) {
      messageText = String(msgObj.text)
    } else if ('message' in msgObj) {
      messageText = String(msgObj.message)
    } else {
      // Fallback: stringify the object
      messageText = JSON.stringify(payload.message)
    }
  }

  return (
    <div className="space-y-2">
      <User className="h-4 w-4 text-muted-foreground" />
      <div className="text-sm text-muted-foreground bg-muted rounded-lg p-3">
        <p className="whitespace-pre-wrap break-words">{messageText}</p>
      </div>
    </div>
  )
}
