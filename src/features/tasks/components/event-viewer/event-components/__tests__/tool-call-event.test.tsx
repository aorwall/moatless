import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import type { Event } from '@/types/event';
import type { MergedEvent } from '@/features/tasks/utils/merge-tool-events';

import { ToolCallEvent } from '../tool-call-event';

describe('ToolCallEvent', () => {
  const createToolCallEvent = (
    toolName: string,
    input: Record<string, unknown>,
    toolResult?: Event
  ): MergedEvent => ({
    event_id: 'test-event',
    type: 'message',
    sub_type: 'tool_call',
    task_id: 'test-task',
    iteration_number: 1,
    timestamp: '2025-01-01T00:00:00Z',
    is_error: false,
    context_tokens: 100,
    num_turns: 1,
    total_cost: 0.01,
    payload: {
      type: 'assistant',
      message: {
        content: [
          {
            type: 'tool_use',
            id: 'tool-123',
            name: toolName,
            input,
          },
        ],
      },
    },
    toolResult,
  });

  it('should render tool call without result', () => {
    const event = createToolCallEvent('Read', {
      file_path: '/test/file.ts',
    });

    render(<ToolCallEvent event={event} />);

    expect(screen.getByText('Read')).toBeInTheDocument();
    expect(screen.getByText('/test/file.ts')).toBeInTheDocument();
  });

  it('should render tool call with result', () => {
    const toolResult: Event = {
      event_id: 'result-event',
      type: 'message',
      sub_type: '',
      task_id: 'test-task',
      iteration_number: 1,
      timestamp: '2025-01-01T00:00:01Z',
      is_error: false,
      context_tokens: 100,
      num_turns: 1,
      total_cost: 0.01,
      payload: {
        type: 'user',
        message: {
          content: [
            {
              type: 'tool_result',
              tool_use_id: 'tool-123',
              content: 'File content line 1\nFile content line 2\nFile content line 3\nFile content line 4',
            },
          ],
        },
      },
    };

    const event = createToolCallEvent('Read', { file_path: '/test/file.ts' }, toolResult);

    render(<ToolCallEvent event={event} />);

    expect(screen.getByText('Tool Result')).toBeInTheDocument();
    // Should show first 3 lines
    expect(screen.getByText(/File content line 1/)).toBeInTheDocument();
    expect(screen.getByText(/File content line 3/)).toBeInTheDocument();
    // Should show "more lines" text
    expect(screen.getByText('... (1 more lines)')).toBeInTheDocument();
  });

  it('should expand tool result when clicking more lines', async () => {
    const user = userEvent.setup();
    
    const toolResult: Event = {
      event_id: 'result-event',
      type: 'message',
      sub_type: '',
      task_id: 'test-task',
      iteration_number: 1,
      timestamp: '2025-01-01T00:00:01Z',
      is_error: false,
      context_tokens: 100,
      num_turns: 1,
      total_cost: 0.01,
      payload: {
        type: 'user',
        message: {
          content: [
            {
              type: 'tool_result',
              tool_use_id: 'tool-123',
              content: 'Line 1\nLine 2\nLine 3\nLine 4\nLine 5',
            },
          ],
        },
      },
    };

    const event = createToolCallEvent('Bash', { command: 'ls -la' }, toolResult);

    render(<ToolCallEvent event={event} />);

    // Initially should not show all lines
    expect(screen.queryByText(/Line 5/)).not.toBeInTheDocument();
    
    // Click on "more lines" text
    await user.click(screen.getByText('... (2 more lines)'));
    
    // Now should show all lines
    expect(screen.getByText(/Line 5/)).toBeInTheDocument();
  });

  it('should use dark background for tool results', () => {
    const toolResult: Event = {
      event_id: 'result-event',
      type: 'message',
      sub_type: '',
      task_id: 'test-task',
      iteration_number: 1,
      timestamp: '2025-01-01T00:00:01Z',
      is_error: false,
      context_tokens: 100,
      num_turns: 1,
      total_cost: 0.01,
      payload: {
        type: 'user',
        message: {
          content: [
            {
              type: 'tool_result',
              tool_use_id: 'tool-123',
              content: 'Test result',
            },
          ],
        },
      },
    };

    const event = createToolCallEvent('Write', { file_path: '/test.ts', content: 'test' }, toolResult);

    render(<ToolCallEvent event={event} />);

    const resultContainer = screen.getByText('Test result').closest('div');
    expect(resultContainer).toHaveClass('bg-zinc-800', 'dark:bg-zinc-900');
  });
});