import type { Event } from './event.js'

// Helper types for transformation
export interface ToolCallGroup {
  toolUseEvent: Event
  toolResultEvent?: Event
  toolUseId: string
}
