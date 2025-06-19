/**
 * Agent event types for the swarm system
 */

export enum AgentEventType {
  MESSAGE = 'message',
  START = 'start',
  STOP = 'stop',
  ERROR = 'error',
  INPUT = 'input',
  RESULT = 'result',
  ENVIRONMENT = 'environment',
}

export interface AgentEvent {
  event_id: string;
  type: AgentEventType | string;
  task_id: string;
  iteration_number: number;
  timestamp: string;
  is_error: boolean;
  sub_type: string;
  context_tokens: number;
  num_turns: number;
  total_cost: number;
  payload: Record<string, unknown>;
}

export interface AgentEventsResponse {
  events: Array<AgentEvent>;
}
