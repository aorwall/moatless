import type { SimpleTaskResponse } from '@shared/types/api'
import type { Event } from '@shared/types/event'
import { EventType } from '@shared/types/event'
import type {
    GitDiffResponse,
    Task,
    TaskIteration,
} from '@shared/types/task'
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
    status: 'open',
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

// Event factories
export function createMockEvent(overrides?: Partial<Event>): Event {
  return {
    eventId: 'event-123',
    type: EventType.MESSAGE,
    agentType: 'test-agent',
    taskId: 'task-123',
    timestamp: new Date('2024-01-01T00:00:00Z'),
    isError: false,
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
    type: EventType.MESSAGE,
    payload: {
      role,
      content,
      model: role === 'assistant' ? 'claude-3-sonnet' : undefined,
    },
  })
}

export function createMockToolEvent(toolName: string, input: any): Event {
  return createMockEvent({
    type: EventType.TOOL,
    payload: {
      toolName: toolName,
      input,
      toolUseId: `tool-${Date.now()}`,
    },
  })
}

// Status factories  
export interface MockWorkspaceStatusResponse {
  status: WorkspaceStatus
  hasUncommittedChanges: boolean
  hasUnmergedCommits: boolean
  isBehindBase: boolean
  gitStatus?: {
    currentBranch: string
    baseBranch: string
    commitsAhead: number
    commitsBehind: number
  }
}

export function createMockWorkspaceStatus(
  overrides?: Partial<MockWorkspaceStatusResponse>,
): MockWorkspaceStatusResponse {
  return {
    status: WorkspaceStatus.IDLE,
    hasUncommittedChanges: false,
    hasUnmergedCommits: false,
    isBehindBase: false,
    ...overrides,
  }
}

export interface MockServerStatus {
  running: boolean
  configured: boolean
  url: string
  pid: number
  uptimeSeconds: number
}

export function createMockServerStatus(overrides?: Partial<MockServerStatus>): MockServerStatus {
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
        timestamp: new Date(Date.now() + i * 1000),
      }),
    )
  }
  return events
}
