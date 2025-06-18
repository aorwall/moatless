/**
 * Event types for the swarm system
 */

// Event Types
export enum EventType {
  // Task events
  TASK_STARTED = 'task_started',
  TASK_ENDED = 'task_ended',
  TASK_ERROR = 'task_error',

  // Agent events
  AGENT_TRIGGERED = 'agent_triggered',
  AGENT_COMPLETED = 'agent_completed',
  AGENT_FAILED = 'agent_failed',

  // System events
  CONNECTION_OPENED = 'connection_opened',
  CONNECTION_CLOSED = 'connection_closed',
  SYSTEM_ERROR = 'system_error',

  // Tool events
  TOOL_INVOKED = 'tool_invoked',
  TOOL_COMPLETED = 'tool_completed',
  TOOL_ERROR = 'tool_error',
}

export enum EventSeverity {
  INFO = 'info',
  WARNING = 'warning',
  ERROR = 'error',
  CRITICAL = 'critical',
}

// Base Event
export interface BaseEvent {
  event_id: string;
  event_type: EventType;
  timestamp: string;
  task_id?: string;
  severity: EventSeverity;
  metadata: Record<string, unknown>;
}

// Task Events
export interface TaskEvent extends BaseEvent {
  event_type:
    | EventType.TASK_STARTED
    | EventType.TASK_ENDED
    | EventType.TASK_ERROR;
  agent_type?: string;
  environment_id?: string;
}

// Agent Events
export interface AgentEvent extends BaseEvent {
  event_type:
    | EventType.AGENT_TRIGGERED
    | EventType.AGENT_COMPLETED
    | EventType.AGENT_FAILED;
  agent_type: string;
  agent_id?: string;
  message?: string;
  error_message?: string;
}

// System Events
export interface SystemEvent extends BaseEvent {
  event_type:
    | EventType.CONNECTION_OPENED
    | EventType.CONNECTION_CLOSED
    | EventType.SYSTEM_ERROR;
  connection_id?: string;
  error_message?: string;
  component?: string;
}

// Tool Events
export interface ToolEvent extends BaseEvent {
  event_type:
    | EventType.TOOL_INVOKED
    | EventType.TOOL_COMPLETED
    | EventType.TOOL_ERROR;
  tool_name: string;
  tool_use_id: string;
  input_params?: Record<string, unknown>;
  output?: unknown;
  error_message?: string;
  duration_ms?: number;
}

// Union type for all events
export type Event = TaskEvent | AgentEvent | SystemEvent | ToolEvent;

// Event response model for API
export interface EventResponse {
  events: Array<Event>;
  total_events: number;
  task_id?: string;
  start_time?: string;
  end_time?: string;
}

// Helper type guards
export function isTaskEvent(event: Event): event is TaskEvent {
  return [
    EventType.TASK_STARTED,
    EventType.TASK_ENDED,
    EventType.TASK_ERROR,
  ].includes(event.event_type);
}

export function isAgentEvent(event: Event): event is AgentEvent {
  return [
    EventType.AGENT_TRIGGERED,
    EventType.AGENT_COMPLETED,
    EventType.AGENT_FAILED,
  ].includes(event.event_type);
}

export function isSystemEvent(event: Event): event is SystemEvent {
  return [
    EventType.CONNECTION_OPENED,
    EventType.CONNECTION_CLOSED,
    EventType.SYSTEM_ERROR,
  ].includes(event.event_type);
}

export function isToolEvent(event: Event): event is ToolEvent {
  return [
    EventType.TOOL_INVOKED,
    EventType.TOOL_COMPLETED,
    EventType.TOOL_ERROR,
  ].includes(event.event_type);
}
