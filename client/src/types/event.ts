export enum AgentEventType {
  MESSAGE = 'message',
  ERROR = 'error',
  RESULT = 'result',
  WORKSPACE = 'workspace',
}

export interface Event {
  event_id: string
  type: AgentEventType | string
  task_id: string
  iteration_number: number
  timestamp: string
  is_error: boolean
  sub_type: string
  context_tokens: number
  num_turns: number
  total_cost: number
  payload: Record<string, unknown>
}
