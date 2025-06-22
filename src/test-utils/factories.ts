import type {
  TaskMetadata,
  TaskIteration,
  ClaudeMessage,
  WorkspaceStatusResponse,
  ServerStatus,
} from '@/types/task';
import type { Event } from '@/types/event';
import type { GitDiffResponse } from '@/types/git-diff';
import type { AgentTriggerResponse } from '@/types/agent';

// Task factories
export function createMockTask(overrides?: Partial<TaskMetadata>): TaskMetadata {
  return {
    task_id: 'task-123',
    title: 'Test Task',
    created_at: '2024-01-01T00:00:00Z',
    last_message_at: '2024-01-01T00:00:00Z',
    agent_type: 'test-agent',
    environment_id: 'test-env',
    is_running: false,
    status: 'completed',
    num_turns: 5,
    total_cost_usd: 0.0234,
    iterations: [],
    ...overrides,
  };
}

export function createMockTaskIteration(
  overrides?: Partial<TaskIteration> & { iteration?: number },
): TaskIteration {
  const iteration = overrides?.iteration || 1;
  return {
    iteration_id: `iter-${iteration}`,
    start_time: '2024-01-01T00:00:00Z',
    end_time: '2024-01-01T00:00:10Z',
    status: 'completed',
    message_count: 3,
    cost_usd: 0.01,
    initial_message: `Test iteration ${iteration} message`,
    error_message: null,
    result: 'Task completed successfully',
    duration_ms: 10000,
    git_diff: null,
    ...overrides,
  };
}

export function createMockClaudeMessage(
  overrides?: Partial<ClaudeMessage>,
): ClaudeMessage {
  return {
    id: 'msg-123',
    role: 'assistant',
    content: 'Test message content',
    model: 'claude-3-sonnet',
    created_at: '2024-01-01T00:00:00Z',
    ...overrides,
  };
}

// Event factories
export function createMockEvent(overrides?: Partial<Event>): Event {
  return {
    event_type: 'message',
    event_index: 0,
    timestamp: '2024-01-01T00:00:00Z',
    session_event_idx: 0,
    data: {
      content: 'Test event content',
    },
    ...overrides,
  };
}

export function createMockMessageEvent(
  content: string,
  role: 'user' | 'assistant' = 'user',
): Event {
  return createMockEvent({
    event_type: 'message',
    data: {
      role,
      content,
      model: role === 'assistant' ? 'claude-3-sonnet' : undefined,
    },
  });
}

export function createMockToolEvent(toolName: string, input: any): Event {
  return createMockEvent({
    event_type: 'tool_use',
    data: {
      tool_name: toolName,
      input,
      tool_use_id: `tool-${Date.now()}`,
    },
  });
}

// Status factories
export function createMockWorkspaceStatus(
  overrides?: Partial<WorkspaceStatusResponse>,
): WorkspaceStatusResponse {
  return {
    status: 'idle',
    message: 'Workspace is idle',
    created_at: '2024-01-01T00:00:00Z',
    ...overrides,
  };
}

export function createMockServerStatus(
  overrides?: Partial<ServerStatus>,
): ServerStatus {
  return {
    running: false,
    port: 8080,
    pid: null,
    logs: [],
    ...overrides,
  };
}

// Git diff factory
export function createMockGitDiff(
  overrides?: Partial<GitDiffResponse>,
): GitDiffResponse {
  return {
    diff: `diff --git a/test.ts b/test.ts
index abc123..def456 100644
--- a/test.ts
+++ b/test.ts
@@ -1,3 +1,3 @@
-const old = 'value';
+const new = 'value';`,
    files_changed: 1,
    insertions: 1,
    deletions: 1,
    ...overrides,
  };
}

// Agent trigger response factory
export function createMockAgentTriggerResponse(
  overrides?: Partial<AgentTriggerResponse>,
): AgentTriggerResponse {
  return {
    success: true,
    message: 'Task created successfully',
    task_id: 'task-123',
    ...overrides,
  };
}

// Bulk factories
export function createMockTaskWithIterations(
  taskOverrides?: Partial<TaskMetadata>,
  iterationCount = 3,
): TaskMetadata {
  const iterations: TaskIteration[] = [];
  for (let i = 1; i <= iterationCount; i++) {
    iterations.push(
      createMockTaskIteration({
        iteration: i,
      } as any),
    );
  }

  return createMockTask({
    iterations,
    ...taskOverrides,
  });
}

export function createMockEventList(count = 5): Event[] {
  const events: Event[] = [];
  for (let i = 0; i < count; i++) {
    events.push(
      createMockEvent({
        event_index: i,
        session_event_idx: i,
        timestamp: new Date(Date.now() + i * 1000).toISOString(),
      }),
    );
  }
  return events;
}