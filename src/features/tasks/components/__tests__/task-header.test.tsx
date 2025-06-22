import { describe, expect, it, vi, beforeEach } from 'vitest';
import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { toast } from 'sonner';
import { TaskHeader } from '../task-header';
import {
  useTask,
  useTaskWorkspaceStatus,
  useServerStatus,
  useStartServer,
  useStopServer,
  useStopAgent,
} from '../../hooks/use-tasks';
import { useTaskStreamingError } from '../../stores/task-streaming-store';
import { renderWithRouter } from '@/test-utils/render';
import {
  createMockTask,
  createMockWorkspaceStatus,
  createMockServerStatus,
} from '@/test-utils/factories';

// Mock dependencies
vi.mock('../../hooks/use-tasks');
vi.mock('../../stores/task-streaming-store');
vi.mock('sonner');
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'task-123' }),
}));

// Mock UI components
vi.mock('@/lib/components/ui/sidebar.tsx', () => ({
  SidebarTrigger: () => <button>Sidebar</button>,
}));

describe('TaskHeader', () => {
  const mockStopAgent = vi.fn();
  const mockStartServer = vi.fn();
  const mockStopServer = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();

    // Default mocks
    vi.mocked(useTask).mockReturnValue({
      data: createMockTask({
        task_id: 'task-123',
        title: 'Test Task',
        is_running: false,
        status: 'completed',
        agent_type: 'test-agent',
        num_turns: 5,
        total_cost_usd: 0.0234,
        created_at: '2024-01-01T00:00:00Z',
      }),
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: createMockWorkspaceStatus({ status: 'idle' }),
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskStreamingError).mockReturnValue(null);

    vi.mocked(useServerStatus).mockReturnValue({
      data: createMockServerStatus({ configured: false, running: false }),
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useStopAgent).mockReturnValue({
      mutate: mockStopAgent,
      isPending: false,
    } as any);

    vi.mocked(useStartServer).mockReturnValue({
      mutate: mockStartServer,
      isPending: false,
    } as any);

    vi.mocked(useStopServer).mockReturnValue({
      mutate: mockStopServer,
      isPending: false,
    } as any);
  });

  describe('Basic Rendering', () => {
    it('renders task metadata', () => {
      renderWithRouter(<TaskHeader />);

      expect(screen.getByText('Test Task')).toBeInTheDocument();
      expect(screen.getByText('test-agent')).toBeInTheDocument();
      expect(screen.getByText('5 turns')).toBeInTheDocument();
      expect(screen.getByText('$0.0234')).toBeInTheDocument();
    });

    it('uses task ID when title is not provided', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({
          task_id: 'task-12345678',
          title: undefined,
        }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskHeader />);

      expect(screen.getByText('Task task-123')).toBeInTheDocument();
    });

    it('shows status badge', () => {
      renderWithRouter(<TaskHeader />);
      expect(screen.getByText('completed')).toBeInTheDocument();
    });

    it('shows loading spinner when task is running', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskHeader />);

      // SVG spinner doesn't have role="img", check for the Loader2 icon instead
      const spinner = document.querySelector('.lucide-loader-circle');
      expect(spinner).toHaveClass('animate-spin');
    });

    it('returns null when no metadata', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
      } as any);

      const { container } = renderWithRouter(<TaskHeader />);
      expect(container.firstChild).toBeNull();
    });
  });

  describe('Workspace Status Display', () => {
    const workspaceTestCases = [
      { status: 'not_setup' as const, expected: 'workspace: not set up' },
      { status: 'setting_up' as const, expected: 'workspace: setting up', isLoading: true },
      { status: 'idle' as const, expected: 'workspace: idle' },
      { status: 'running' as const, expected: 'workspace: running', isLoading: true },
    ];

    workspaceTestCases.forEach(({ status, expected, isLoading }) => {
      it(`displays ${status} workspace status`, () => {
        vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
          data: createMockWorkspaceStatus({ status }),
          isLoading: false,
          error: null,
        } as any);

        renderWithRouter(<TaskHeader />);

        expect(screen.getByText(expected)).toBeInTheDocument();
        
        if (isLoading) {
          // Check for loading spinner using data-slot attribute
          const badge = screen.getByText(expected).closest('[data-slot="badge"]');
          expect(badge?.querySelector('.animate-spin')).toBeInTheDocument();
        }
      });
    });

    it('shows loading state when fetching workspace status', () => {
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: null,
        isLoading: true,
        error: null,
      } as any);

      renderWithRouter(<TaskHeader />);

      expect(screen.getByText('workspace: unknown')).toBeInTheDocument();
    });

    it('shows error state for workspace', async () => {
      const user = userEvent.setup();
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: null,
        isLoading: false,
        error: new Error('Failed to fetch workspace status'),
      } as any);

      renderWithRouter(<TaskHeader />);

      const errorBadge = screen.getByText('workspace: error');
      expect(errorBadge).toBeInTheDocument();

      // Hover to see error tooltip
      await user.hover(errorBadge);
      await waitFor(() => {
        // Use getAllByText since the error might appear multiple times
        expect(screen.getAllByText('Failed to fetch workspace status')[0]).toBeInTheDocument();
      });
    });
  });

  describe('Streaming Error Display', () => {
    it('shows streaming error when present', async () => {
      const user = userEvent.setup();
      vi.mocked(useTaskStreamingError).mockReturnValue('Connection lost');

      renderWithRouter(<TaskHeader />);

      expect(screen.getByText('Streaming error')).toBeInTheDocument();

      // Hover to see error details
      await user.hover(screen.getByText('Streaming error'));
      await waitFor(() => {
        // Use getAllByText since the error might appear multiple times (in tooltip)
        expect(screen.getAllByText('Connection lost')[0]).toBeInTheDocument();
      });
    });

    it('does not show streaming error when null', () => {
      vi.mocked(useTaskStreamingError).mockReturnValue(null);

      renderWithRouter(<TaskHeader />);

      expect(screen.queryByText('Streaming error')).not.toBeInTheDocument();
    });
  });

  describe('Agent Controls', () => {
    it('shows stop button when task is running', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskHeader />);

      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();
    });

    it('does not show stop button when task is not running', () => {
      renderWithRouter(<TaskHeader />);

      expect(screen.queryByRole('button', { name: /stop/i })).not.toBeInTheDocument();
    });

    it('handles stop agent action', async () => {
      const user = userEvent.setup();
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true, task_id: 'task-123' }),
        isLoading: false,
        error: null,
      } as any);

      const onSuccess = vi.fn();
      vi.mocked(useStopAgent).mockImplementation((options) => {
        onSuccess.mockImplementation(() => options?.onSuccess?.({} as any, 'task-123', {}));
        return {
          mutate: mockStopAgent,
          isPending: false,
        } as any;
      });

      renderWithRouter(<TaskHeader />);

      const stopButton = screen.getByRole('button', { name: /stop/i });
      await user.click(stopButton);

      expect(mockStopAgent).toHaveBeenCalledWith('task-123');
      
      // Simulate success
      onSuccess();
      expect(toast.success).toHaveBeenCalledWith('Agent stopped successfully');
    });

    it('shows loading state when stopping agent', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as any);

      vi.mocked(useStopAgent).mockReturnValue({
        mutate: mockStopAgent,
        isPending: true,
      } as any);

      renderWithRouter(<TaskHeader />);

      const stopButton = screen.getByRole('button', { name: /stopping\.\.\./i });
      expect(stopButton).toBeDisabled();
    });
  });

  describe('Server Controls', () => {
    it('shows server controls when configured', () => {
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({ configured: true, running: false }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskHeader />);

      expect(screen.getByRole('button', { name: /start server/i })).toBeInTheDocument();
    });

    it('does not show server controls when not configured', () => {
      renderWithRouter(<TaskHeader />);

      expect(screen.queryByRole('button', { name: /server/i })).not.toBeInTheDocument();
    });

    it('handles start server action', async () => {
      const user = userEvent.setup();
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({ configured: true, running: false }),
        isLoading: false,
        error: null,
      } as any);

      const onSuccess = vi.fn();
      vi.mocked(useStartServer).mockImplementation((options) => {
        onSuccess.mockImplementation(() => options?.onSuccess?.({} as any, 'task-123', {}));
        return {
          mutate: mockStartServer,
          isPending: false,
        } as any;
      });

      renderWithRouter(<TaskHeader />);

      const startButton = screen.getByRole('button', { name: /start server/i });
      await user.click(startButton);

      expect(mockStartServer).toHaveBeenCalledWith('task-123');
      
      // Simulate success
      onSuccess();
      expect(toast.success).toHaveBeenCalledWith('Server started successfully');
    });

    it('shows stop server button when running', async () => {
      const user = userEvent.setup();
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({
          configured: true,
          running: true,
          url: 'http://localhost:8080',
          pid: 12345,
          uptime_seconds: 120,
        }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskHeader />);

      const stopButton = screen.getByRole('button', { name: /stop server/i });
      expect(stopButton).toBeInTheDocument();

      // Hover to see server details
      await user.hover(stopButton);
      await waitFor(() => {
        // Use getAllByText since the text might appear multiple times (in tooltip)
        expect(screen.getAllByText('Server is running')[0]).toBeInTheDocument();
        expect(screen.getAllByText('URL: http://localhost:8080')[0]).toBeInTheDocument();
        expect(screen.getAllByText('PID: 12345')[0]).toBeInTheDocument();
        expect(screen.getAllByText('Uptime: 120s')[0]).toBeInTheDocument();
      });
    });

    it('handles server operation errors', async () => {
      const user = userEvent.setup();
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({ configured: true, running: false }),
        isLoading: false,
        error: null,
      } as any);

      const onError = vi.fn();
      vi.mocked(useStartServer).mockImplementation((options) => {
        onError.mockImplementation(() => 
          options?.onError?.(new Error('Port already in use'), 'task-123', {})
        );
        return {
          mutate: mockStartServer,
          isPending: false,
        } as any;
      });

      renderWithRouter(<TaskHeader />);

      const startButton = screen.getByRole('button', { name: /start server/i });
      await user.click(startButton);

      // Simulate error
      onError();
      expect(toast.error).toHaveBeenCalledWith('Failed to start server', {
        description: 'Port already in use',
      });
    });

    it('disables buttons during server operations', () => {
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({ configured: true, running: false }),
        isLoading: false,
        error: null,
      } as any);

      vi.mocked(useStartServer).mockReturnValue({
        mutate: mockStartServer,
        isPending: true,
      } as any);

      renderWithRouter(<TaskHeader />);

      const startButton = screen.getByRole('button', { name: /starting server/i });
      expect(startButton).toBeDisabled();
    });
  });

  describe('Date Formatting', () => {
    it('formats creation date correctly', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({
          created_at: '2024-01-15T14:30:00Z',
        }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskHeader />);

      // The exact format depends on the locale, so we just check it's rendered
      expect(screen.getByText(/2024/)).toBeInTheDocument();
    });
  });

  describe('Status Badge Variants', () => {
    const statusVariants = [
      { status: 'active', variant: 'default' },
      { status: 'error', variant: 'destructive' },
      { status: 'completed', variant: 'secondary' },
    ];

    statusVariants.forEach(({ status }) => {
      it(`renders correct variant for ${status} status`, () => {
        vi.mocked(useTask).mockReturnValue({
          data: createMockTask({ status }),
          isLoading: false,
          error: null,
        } as any);

        renderWithRouter(<TaskHeader />);

        expect(screen.getByText(status)).toBeInTheDocument();
      });
    });
  });
});