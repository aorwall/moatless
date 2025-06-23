import { screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { useTask } from '@/features/tasks/hooks/use-tasks';
import { createMockTaskWithIterations } from '@/test-utils/factories';
import { renderWithRouter } from '@/test-utils/render';

import { TaskViewer } from '../task-viewer';

// Mock dependencies
vi.mock('@/features/tasks/hooks/use-tasks');
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'task-123' }),
}));
vi.mock('@/features/tasks/stores/task-ui-store', () => ({
  useTaskUIStore: vi.fn((selector) => {
    const state = {
      setSelectedIteration: vi.fn(),
    };
    return selector ? selector(state) : state;
  }),
}));

describe('TaskViewer Refetch Behavior', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Mock scrollIntoView
    Element.prototype.scrollIntoView = vi.fn();
  });

  it('should display updated task data when useTask returns new data', async () => {
    // Initial task with 1 iteration
    const initialTask = createMockTaskWithIterations(
      {
        task_id: 'task-123',
        title: 'Test Task',
        is_running: false,
        status: 'completed',
      },
      1,
    );

    // Updated task with 2 iterations
    const updatedTask = createMockTaskWithIterations(
      {
        task_id: 'task-123',
        title: 'Test Task',
        is_running: true,
        status: 'active',
      },
      2,
    );

    // Start with initial task
    vi.mocked(useTask).mockReturnValue({
      data: initialTask,
      isLoading: false,
      error: null,
    } as any);

    const { rerender } = renderWithRouter(<TaskViewer />);

    // Verify initial state - only 1 iteration
    expect(
      screen.getByText(initialTask.iterations[0].initial_message),
    ).toBeInTheDocument();
    expect(screen.queryByText('Processing...')).not.toBeInTheDocument();

    // Update the mock to return updated task
    vi.mocked(useTask).mockReturnValue({
      data: updatedTask,
      isLoading: false,
      error: null,
    } as any);

    // Re-render the component
    rerender(<TaskViewer />);

    // Wait for the UI to update
    await waitFor(() => {
      // Should now show both iterations
      expect(
        screen.getByText(updatedTask.iterations[0].initial_message),
      ).toBeInTheDocument();
      expect(
        screen.getByText(updatedTask.iterations[1].initial_message),
      ).toBeInTheDocument();
      // Should show processing indicator when task is running
      expect(screen.getByText('Processing...')).toBeInTheDocument();
    });
  });

  it('should handle null task data gracefully', () => {
    vi.mocked(useTask).mockReturnValue({
      data: null,
      isLoading: false,
      error: null,
    } as any);

    const { container } = renderWithRouter(<TaskViewer />);

    // Should render nothing
    expect(container.firstChild).toBeNull();
  });

  it('should display loading indicator only for running tasks with iterations', () => {
    const runningTask = createMockTaskWithIterations(
      {
        task_id: 'task-123',
        is_running: true,
        status: 'active',
      },
      2,
    );

    vi.mocked(useTask).mockReturnValue({
      data: runningTask,
      isLoading: false,
      error: null,
    } as any);

    renderWithRouter(<TaskViewer />);

    // Should show the processing indicator
    expect(screen.getByText('Processing...')).toBeInTheDocument();
    // Check for the spinner by its class
    const spinner = screen.getByText('Processing...').previousElementSibling;
    expect(spinner).toHaveClass('animate-spin');
  });

  it('should update cost and duration when task data changes', async () => {
    const task1 = createMockTaskWithIterations({ task_id: 'task-123' }, 1);
    task1.iterations[0].cost_usd = 0.01;
    task1.iterations[0].duration_ms = 1000;

    const task2 = createMockTaskWithIterations({ task_id: 'task-123' }, 1);
    task2.iterations[0].cost_usd = 0.02;
    task2.iterations[0].duration_ms = 2000;

    // Start with task1
    vi.mocked(useTask).mockReturnValue({
      data: task1,
      isLoading: false,
      error: null,
    } as any);

    const { rerender } = renderWithRouter(<TaskViewer />);

    // Verify initial cost and duration
    expect(screen.getByText('$0.0100')).toBeInTheDocument();
    expect(screen.getByText('0m 1s')).toBeInTheDocument();

    // Update to task2
    vi.mocked(useTask).mockReturnValue({
      data: task2,
      isLoading: false,
      error: null,
    } as any);

    rerender(<TaskViewer />);

    await waitFor(() => {
      // Verify updated cost and duration
      expect(screen.getByText('$0.0200')).toBeInTheDocument();
      expect(screen.getByText('0m 2s')).toBeInTheDocument();
    });
  });
});
