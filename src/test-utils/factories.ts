import type { AgentTriggerResponse } from '@/types/agent';
import type { Event } from '@/types/event';
import type { GitDiffResponse } from '@/types/git-diff';
import type {
  AssistantMessage,
  ClaudeMessage,
  ServerStatus,
  TaskIteration,
  TaskMetadata,
  WorkspaceStatusResponse,
} from '@/types/task';
import { ContentType, MessageType, Role, WorkspaceStatus } from '@/types/task';

// Task factories
export function createMockTask(
  overrides?: Partial<TaskMetadata>,
): TaskMetadata {
  return {
    task_id: 'task-123',
    title: 'Test Task',
    created_at: '2024-01-01T00:00:00Z',
    last_message_at: '2024-01-01T00:00:00Z',
    agent_type: 'test-agent',
    environment_id: 'test-env',
    environment_name: 'test-env-name',
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
    type: MessageType.ASSISTANT,
    task_id: 'task-123',
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
  } as AssistantMessage;
}

// Event factories
export function createMockEvent(overrides?: Partial<Event>): Event {
  return {
    event_id: 'event-123',
    type: 'message',
    task_id: 'task-123',
    iteration_number: 1,
    timestamp: '2024-01-01T00:00:00Z',
    is_error: false,
    sub_type: 'message',
    context_tokens: 100,
    num_turns: 1,
    total_cost: 0.001,
    payload: {
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
    type: 'message',
    payload: {
      role,
      content,
      model: role === 'assistant' ? 'claude-3-sonnet' : undefined,
    },
  });
}

export function createMockToolEvent(toolName: string, input: any): Event {
  return createMockEvent({
    type: 'tool_use',
    payload: {
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
    status: WorkspaceStatus.IDLE,
    has_uncommitted_changes: false,
    ...overrides,
  };
}

export function createMockServerStatus(
  overrides?: Partial<ServerStatus>,
): ServerStatus {
  return {
    running: false,
    configured: true,
    url: 'http://localhost:8080',
    pid: 1234,
    uptime_seconds: 0,
    ...overrides,
  };
}

// Git diff factory
export function createMockGitDiff(
  overrides?: Partial<GitDiffResponse>,
): GitDiffResponse {
  const defaultDiff = `diff --git a/test.ts b/test.ts
index 1234567..abcdefg 100644
--- a/test.ts
+++ b/test.ts
@@ -1 +1 @@
-const old = 'value';
+const new = 'value';`;

  return {
    files: [],
    total_files: 1,
    total_additions: 1,
    total_deletions: 1,
    diff: defaultDiff,
    ...overrides,
  };
}

// Agent trigger response factory
export function createMockAgentTriggerResponse(
  overrides?: Partial<AgentTriggerResponse>,
): AgentTriggerResponse {
  return {
    task_id: 'task-123',
    status: 'created',
    stream_url: 'ws://localhost:8080/stream',
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
        event_id: `event-${i}`,
        timestamp: new Date(Date.now() + i * 1000).toISOString(),
      }),
    );
  }
  return events;
}
