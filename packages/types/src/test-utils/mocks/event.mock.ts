import type {
  AgentEvent,
  Event,
  EventListResponse,
  ServerEvent,
  StreamStatusResponse,
  TaskEvent,
  TaskIterationEvent,
  WorkspaceEvent,
} from '../../types/event.js'

export function createMockEvent(overrides?: Partial<Event>): Event {
  return {
    eventId: 'event-123',
    agentType: 'claude-code',
    type: 'agent',
    subType: 'message',
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: false,
    payload: { message: 'Test event payload' },
    ...overrides,
  }
}

export function createMockAgentMessageEvent(overrides?: Partial<AgentEvent>): AgentEvent {
  return {
    eventId: 'message-event-123',
    agentType: 'claude-code',
    type: 'agent',
    subType: 'message',
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: false,
    payload: {
      role: 'assistant',
      content: [
        {
          type: 'text',
          text: 'Hello, I can help you with that.',
        },
      ],
    },
    ...overrides,
  }
}

export function createMockAgentErrorEvent(
  error: string,
  overrides?: Partial<AgentEvent>,
): AgentEvent {
  return {
    eventId: 'error-event-123',
    agentType: 'claude-code',
    type: 'agent',
    subType: 'error',
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: true,
    payload: {
      error,
      stackTrace: 'Error stack trace here',
      code: 'MOCK_ERROR',
    },
    ...overrides,
  }
}

export function createMockWorkspaceEvent(overrides?: Partial<WorkspaceEvent>): WorkspaceEvent {
  return {
    eventId: 'workspace-event-123',
    agentType: 'claude-code',
    type: 'workspace',
    subType: 'ready',
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: false,
    payload: {
      action: 'file_created',
      path: '/workspace/test.ts',
      details: 'Created new TypeScript file',
    },
    ...overrides,
  }
}

export function createMockServerEvent(overrides?: Partial<ServerEvent>): ServerEvent {
  return {
    eventId: 'server-event-123',
    agentType: 'claude-code',
    type: 'server',
    subType: 'started',
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: false,
    payload: {
      status: 'started',
      previousStatus: 'starting',
      port: 3000,
      url: 'http://localhost:3000',
    },
    ...overrides,
  }
}

export function createMockAgentCompletedEvent(overrides?: Partial<AgentEvent>): AgentEvent {
  return {
    eventId: 'result-event-123',
    agentType: 'claude-code',
    type: 'agent',
    subType: 'completed',
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: false,
    payload: {
      success: true,
      result: 'Task completed successfully',
      output: 'Build completed with no errors',
    },
    ...overrides,
  }
}

export function createMockTaskEvent(
  status: 'created' | 'open' | 'closed',
  overrides?: Partial<TaskEvent>,
): TaskEvent {
  return {
    eventId: 'task-event-123',
    agentType: 'claude-code',
    type: 'task',
    subType: status,
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: false,
    payload: {
      status,
      previousStatus: 'open',
    },
    ...overrides,
  }
}

export function createMockTaskIterationEvent(
  status: 'created' | 'active' | 'completed' | 'error',
  overrides?: Partial<TaskIterationEvent>,
): TaskIterationEvent {
  return {
    eventId: 'task-iteration-event-123',
    agentType: 'claude-code',
    type: 'taskIteration',
    subType: status,
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: status === 'error',
    payload: {
      status,
      previousStatus: 'created',
      iterationNumber: 1,
    },
    ...overrides,
  }
}

export function createMockEventListResponse(eventCount = 5): EventListResponse {
  const items: Event[] = []

  for (let i = 0; i < eventCount; i++) {
    const eventTypes = ['agent', 'workspace', 'server', 'task'] as const
    const type = eventTypes[i % eventTypes.length]

    items.push(
      createMockEvent({
        eventId: `event-${i + 1}`,
        type,
        timestamp: new Date(`2024-01-01T00:${String(i).padStart(2, '0')}:00Z`),
        payload: { message: `Event ${i + 1} payload` },
      }),
    )
  }

  return {
    items,
    pagination: {
      limit: 20,
      offset: 0,
      total: eventCount,
      hasMore: false,
    },
  }
}

export function createMockStreamStatusResponse(
  overrides?: Partial<StreamStatusResponse>,
): StreamStatusResponse {
  return {
    globalConnections: 5,
    globalEventCount: 150,
    taskConnections: {
      'task-123': 2,
      'task-456': 1,
    },
    taskEventCounts: {
      'task-123': 50,
      'task-456': 25,
    },
    totalConnections: 8,
    totalEventCount: 225,
    ...overrides,
  }
}

// Helper function to create an async generator of events for testing SSE
export async function* createMockEventStream(
  events: Event[],
  delayMs = 100,
): AsyncGenerator<Event> {
  for (const event of events) {
    yield event
    await new Promise((resolve) => setTimeout(resolve, delayMs))
  }
}
