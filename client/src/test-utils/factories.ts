import type { SimpleTaskResponse } from '@shared/types/api'
import type { Event } from '@shared/types/event'
import type { GitDiffResponse } from '@shared/types/git-diff'
import type {
  AssistantMessage,
  ClaudeMessage,
  ServerStatus,
  Task,
  TaskIteration,
  WorkspaceStatusResponse,
} from '@shared/types/task'
import { ContentType, MessageType, Role } from '@shared/types/task'
import { WorkspaceStatus } from '@shared/types/workspace'

// Re-export environment factories
export * from './factories/environment'

// Task factories
export function createMockTask(overrides?: Partial<Task>): Task {
  return {
    taskId: 'task-123',
    createdAt: new Date('2024-01-01T00:00:00Z'),
    lastMessageAt: new Date('2024-01-01T00:00:00Z'),
    agentType: 'test-agent',
    environmentId: 'test-env',
    status: 'completed',
    numTurns: 5,
    totalCostUsd: 0.0234,
    iterations: [],
    ...overrides,
  }
}

export function createMockTaskIteration(
  overrides?: Partial<TaskIteration> & { iteration?: number },
): TaskIteration {
  const iteration = overrides?.iteration || 1
  return {
    iterationNumber: iteration,
    startTime: new Date('2024-01-01T00:00:00Z'),
    endTime: new Date('2024-01-01T00:00:10Z'),
    status: 'completed',
    messageCount: 3,
    costUsd: 0.01,
    initialMessage: `Test iteration ${iteration} message`,
    result: 'Task completed successfully',
    durationMs: 10000,
    gitDiff: undefined,
    ...overrides,
  }
}

export function createMockClaudeMessage(overrides?: Partial<ClaudeMessage>): ClaudeMessage {
  return {
    type: MessageType.ASSISTANT,
    taskId: 'task-123',
    timestamp: '2024-01-01T00:00:00Z',
    message: {
      type: 'message',
      role: Role.ASSISTANT,
      content: [
        {
          type: ContentType.TEXT,
          text: 'Test message content',
        },
      ],
      model: 'claude-3-sonnet',
    },
    ...overrides,
  } as AssistantMessage
}

// Event factories
export function createMockEvent(overrides?: Partial<Event>): Event {
  return {
    eventId: 'event-123',
    type: 'message',
    taskId: 'task-123',
    iterationNumber: 1,
    timestamp: '2024-01-01T00:00:00Z',
    isError: false,
    subType: 'message',
    contextTokens: 100,
    numTurns: 1,
    totalCost: 0.001,
    payload: {
      content: 'Test event content',
    },
    ...overrides,
  }
}

export function createMockMessageEvent(
  content: string,
  role: 'user' | 'assistant' = 'user',
): Event {
  return createMockEvent({
    type: 'message',
    payload: {
      role,
      content,
      model: role === 'assistant' ? 'claude-3-sonnet' : undefined,
    },
  })
}

export function createMockToolEvent(toolName: string, input: any): Event {
  return createMockEvent({
    type: 'toolUse',
    payload: {
      toolName: toolName,
      input,
      toolUseId: `tool-${Date.now()}`,
    },
  })
}

// Status factories
export function createMockWorkspaceStatus(
  overrides?: Partial<WorkspaceStatusResponse>,
): WorkspaceStatusResponse {
  return {
    status: WorkspaceStatus.IDLE,
    hasUncommittedChanges: false,
    ...overrides,
  }
}

export function createMockServerStatus(overrides?: Partial<ServerStatus>): ServerStatus {
  return {
    running: false,
    configured: true,
    url: 'http://localhost:8080',
    pid: 1234,
    uptimeSeconds: 0,
    ...overrides,
  }
}

// Git diff factory
export function createMockGitDiff(overrides?: Partial<GitDiffResponse>): GitDiffResponse {
  const defaultDiff = `diff --git a/test.ts b/test.ts
index 1234567..abcdefg 100644
--- a/test.ts
+++ b/test.ts
@@ -1 +1 @@
-const old = 'value';
+const new = 'value';`

  return {
    files: [],
    diff: defaultDiff,
    ...overrides,
  }
}

// Agent trigger response factory
export function createMockAgentTriggerResponse(
  overrides?: Partial<SimpleTaskResponse>,
): SimpleTaskResponse {
  return {
    taskId: 'task-123',
    status: 'created',
    ...overrides,
  }
}

// Bulk factories
export function createMockTaskWithIterations(
  taskOverrides?: Partial<Task>,
  iterationCount = 3,
): Task {
  const iterations: TaskIteration[] = []
  for (let i = 1; i <= iterationCount; i++) {
    iterations.push(
      createMockTaskIteration({
        iteration: i,
      } as any),
    )
  }

  return createMockTask({
    iterations,
    ...taskOverrides,
  })
}

export function createMockEventList(count = 5): Event[] {
  const events: Event[] = []
  for (let i = 0; i < count; i++) {
    events.push(
      createMockEvent({
        eventId: `event-${i}`,
        timestamp: new Date(Date.now() + i * 1000).toISOString(),
      }),
    )
  }
  return events
}
