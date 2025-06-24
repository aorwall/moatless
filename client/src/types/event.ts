export enum AgentEventType {
  MESSAGE = 'message',
  ERROR = 'error',
  RESULT = 'result',
  WORKSPACE = 'workspace',
}

export interface Event {
  event_id: string
  type: AgentEventType | string
  taskId: string
  iterationNumber: number
  timestamp: string
  isError: boolean
  subType: string
  contextTokens: number
  numTurns: number
  totalCost: number
  payload: Record<string, unknown>
}
