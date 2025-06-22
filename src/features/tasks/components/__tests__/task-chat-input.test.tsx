import { screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { createMockTask } from '@/test-utils/factories';
import { renderWithRouter } from '@/test-utils/render';

import { useTask } from '../../hooks/use-tasks';
import { TaskChatInput } from '../task-chat-input';

// Mock dependencies
vi.mock('../../hooks/use-tasks');
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'task-123' }),
}));

// Mock ChatInput component
vi.mock('../chat-input', () => ({
  ChatInput: ({ taskId, taskStatus, className }: any) => {
    const attrs: any = {
      'data-testid': 'chat-input',
      'data-task-id': taskId,
    };

    // Only set data-task-status if taskStatus is defined
    if (taskStatus !== undefined && taskStatus !== null) {
      attrs['data-task-status'] = taskStatus;
    }

    return (
      <div {...attrs} className={className}>
        ChatInput Component
      </div>
    );
  },
}));

describe('TaskChatInput', () => {
  beforeEach(() => {
    vi.clearAllMocks();

    // Default mock
    vi.mocked(useTask).mockReturnValue({
      data: createMockTask({
        task_id: 'task-123',
        status: 'active',
      }),
      isLoading: false,
      error: null,
    } as any);
  });

  describe('Props Passing', () => {
    it('passes taskId from params to ChatInput', () => {
      renderWithRouter(<TaskChatInput />);

      const chatInput = screen.getByTestId('chat-input');
      expect(chatInput).toHaveAttribute('data-task-id', 'task-123');
    });

    it('passes task status to ChatInput', () => {
      renderWithRouter(<TaskChatInput />);

      const chatInput = screen.getByTestId('chat-input');
      expect(chatInput).toHaveAttribute('data-task-status', 'active');
    });

    it('passes className prop to ChatInput', () => {
      renderWithRouter(<TaskChatInput className="custom-class" />);

      const chatInput = screen.getByTestId('chat-input');
      expect(chatInput).toHaveClass('custom-class');
    });

    it('handles undefined task status', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskChatInput />);

      const chatInput = screen.getByTestId('chat-input');
      expect(chatInput).not.toHaveAttribute('data-task-status');
    });
  });

  describe('Task Status Integration', () => {
    const statuses = ['active', 'completed', 'error'] as const;

    statuses.forEach((status) => {
      it(`passes ${status} status to ChatInput`, () => {
        vi.mocked(useTask).mockReturnValue({
          data: createMockTask({ status }),
          isLoading: false,
          error: null,
        } as any);

        renderWithRouter(<TaskChatInput />);

        const chatInput = screen.getByTestId('chat-input');
        expect(chatInput).toHaveAttribute('data-task-status', status);
      });
    });
  });

  describe('Loading States', () => {
    it('renders while task is loading', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: true,
        error: null,
      } as any);

      renderWithRouter(<TaskChatInput />);

      // Component should still render
      expect(screen.getByTestId('chat-input')).toBeInTheDocument();
    });

    it('renders when task has error', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: new Error('Failed to load task'),
      } as any);

      renderWithRouter(<TaskChatInput />);

      // Component should still render
      expect(screen.getByTestId('chat-input')).toBeInTheDocument();
    });
  });

  describe('Component Integration', () => {
    it('renders ChatInput component', () => {
      renderWithRouter(<TaskChatInput />);

      expect(screen.getByText('ChatInput Component')).toBeInTheDocument();
    });

    it('maintains consistent props when task updates', () => {
      const { rerender } = renderWithRouter(<TaskChatInput />);

      let chatInput = screen.getByTestId('chat-input');
      expect(chatInput).toHaveAttribute('data-task-id', 'task-123');
      expect(chatInput).toHaveAttribute('data-task-status', 'active');

      // Update task status
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({
          task_id: 'task-123',
          status: 'completed',
        }),
        isLoading: false,
        error: null,
      } as any);

      rerender(<TaskChatInput />);

      // Re-query the element after rerender
      chatInput = screen.getByTestId('chat-input');
      // Task ID should remain the same
      expect(chatInput).toHaveAttribute('data-task-id', 'task-123');
      // Status should update
      expect(chatInput).toHaveAttribute('data-task-status', 'completed');
    });
  });

  describe('Edge Cases', () => {
    it('handles task with minimal data', () => {
      vi.mocked(useTask).mockReturnValue({
        data: { task_id: 'task-123' } as any,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskChatInput />);

      const chatInput = screen.getByTestId('chat-input');
      expect(chatInput).toHaveAttribute('data-task-id', 'task-123');
      expect(chatInput).not.toHaveAttribute('data-task-status');
    });

    it('handles className as undefined', () => {
      renderWithRouter(<TaskChatInput />);

      const chatInput = screen.getByTestId('chat-input');
      expect(chatInput).not.toHaveClass('undefined');
    });

    it('handles empty className', () => {
      renderWithRouter(<TaskChatInput className="" />);

      const chatInput = screen.getByTestId('chat-input');
      // Should not have any class when className is empty
      expect(chatInput.className).toBe('');
    });
  });

  describe('Wrapper Behavior', () => {
    it('acts as a thin wrapper around ChatInput', () => {
      const task = createMockTask({
        task_id: 'task-456',
        status: 'active',
      });

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskChatInput className="test-class" />);

      const chatInput = screen.getByTestId('chat-input');

      // Verify all props are passed through correctly
      expect(chatInput).toHaveAttribute('data-task-id', 'task-123'); // This uses mocked params
      expect(chatInput).toHaveAttribute('data-task-status', 'active');
      expect(chatInput).toHaveClass('test-class');
    });

    it('does not add extra wrapper elements', () => {
      const { container } = renderWithRouter(<TaskChatInput />);

      // TaskChatInput should render ChatInput without adding additional DOM nodes
      const chatInput = container.querySelector('[data-testid="chat-input"]');
      expect(chatInput).toBeInTheDocument();
      // The chatInput should be the only child of the container
      expect(container.children).toHaveLength(1);
      expect(container.firstElementChild).toBe(chatInput);
    });
  });
});
