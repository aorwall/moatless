import type { Event } from '../schemas/event.schemas.js'

// Helper types for transformation
export interface ToolCallGroup {
  toolUseEvent: Event
  toolResultEvent?: Event
  toolUseId: string
}
