import type {
  ErrorEvent,
  Event,
  EventListResponse,
  MessageEvent,
  ResultEvent,
  ServerEvent,
  StreamStatusResponse,
  TaskStatusEvent,
  WorkspaceEvent,
} from '../../types/event.js'

export function createMockEvent(overrides?: Partial<Event>): Event {
  return {
    eventId: 'event-123',
    agentType: 'claude-code',
    type: 'message',
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: false,
    payload: { message: 'Test event payload' },
    ...overrides,
  }
}

export function createMockMessageEvent(overrides?: Partial<MessageEvent>): MessageEvent {
  return {
    eventId: 'message-event-123',
    agentType: 'claude-code',
    type: 'message',
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

export function createMockErrorEvent(error: string, overrides?: Partial<ErrorEvent>): ErrorEvent {
  return {
    eventId: 'error-event-123',
    agentType: 'claude-code',
    type: 'error',
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
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: false,
    payload: {
      action: 'server_started',
      port: 3000,
      url: 'http://localhost:3000',
    },
    ...overrides,
  }
}

export function createMockResultEvent(overrides?: Partial<ResultEvent>): ResultEvent {
  return {
    eventId: 'result-event-123',
    agentType: 'claude-code',
    type: 'result',
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

export function createMockTaskStatusEvent(
  status: string,
  overrides?: Partial<TaskStatusEvent>,
): TaskStatusEvent {
  return {
    eventId: 'task-status-event-123',
    agentType: 'claude-code',
    type: 'task_status',
    subType: status,
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: false,
    payload: {
      status,
      previousStatus: 'open',
      iterationNumber: 1,
    },
    ...overrides,
  }
}

export function createMockEventListResponse(eventCount = 5): EventListResponse {
  const items: Event[] = []

  for (let i = 0; i < eventCount; i++) {
    const eventTypes = ['message', 'workspace', 'server', 'result'] as const
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
