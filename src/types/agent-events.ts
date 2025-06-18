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
}

export interface AgentEvent {
  event_id: string;
  type: AgentEventType;
  agent_session_id: string;
  timestamp: string;
  is_error: boolean;
  payload: Record<string, unknown>;
}

export interface AgentEventsResponse {
  events: Array<AgentEvent>;
}
