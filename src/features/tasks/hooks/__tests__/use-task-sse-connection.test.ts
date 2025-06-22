import { describe, expect, it, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { useTaskSSEConnection } from '../use-task-sse-connection';
import { useTask, useTaskEvents, useTaskWorkspaceStatus } from '../use-tasks';
import { useTaskStreamingStore } from '../../stores/task-streaming-store';
import { createWrapper } from '@/test-utils/render';
import {
  createMockTask,
  createMockEventList,
  createMockWorkspaceStatus,
} from '@/test-utils/factories';

// Mock dependencies
vi.mock('../use-tasks');
vi.mock('../use-sse');
vi.mock('../../stores/task-streaming-store');

describe('useTaskSSEConnection', () => {
  const mockTaskId = 'task-123';
  
  // Mock store actions
  const mockStoreActions = {
    addLocalMessage: vi.fn(),
    addStreamEvent: vi.fn(),
    setStreamEvents: vi.fn(),
    setStreamingError: vi.fn(),
    clearLocalMessages: vi.fn(),
    clearStreamEvents: vi.fn(),
    clearTaskStreaming: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
    
    // Default mocks
    vi.mocked(useTask).mockReturnValue({
      data: createMockTask({ id: mockTaskId, is_running: false, status: 'completed' }),
      isLoading: false,
      error: null,
    } as any);
    
    vi.mocked(useTaskEvents).mockReturnValue({
      data: createMockEventList(3),
      isLoading: false,
      error: null,
    } as any);
    
    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: createMockWorkspaceStatus({ status: 'idle' }),
      isLoading: false,
      error: null,
    } as any);
    
    vi.mocked(useTaskStreamingStore).mockReturnValue(mockStoreActions);
  });

  describe('Streaming Conditions', () => {
    it('should stream when task is running', async () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true, status: 'active' }),
        isLoading: false,
        error: null,
      } as any);

      const { result } = renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      expect(result.current.isStreaming).toBe(true);
    });

    it('should stream when workspace is running', async () => {
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: createMockWorkspaceStatus({ status: 'running' }),
        isLoading: false,
        error: null,
      } as any);

      const { result } = renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      expect(result.current.isStreaming).toBe(true);
    });

    it('should stream when workspace is setting up', async () => {
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: createMockWorkspaceStatus({ status: 'setting_up' }),
        isLoading: false,
        error: null,
      } as any);

      const { result } = renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      expect(result.current.isStreaming).toBe(true);
    });

    it('should not stream when both task and workspace are inactive', async () => {
      // Already set up with inactive defaults
      const { result } = renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      expect(result.current.isStreaming).toBe(false);
    });

    it('should stream when either task OR workspace is active', async () => {
      // Task inactive but workspace running
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: false, status: 'completed' }),
        isLoading: false,
        error: null,
      } as any);
      
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: createMockWorkspaceStatus({ status: 'running' }),
        isLoading: false,
        error: null,
      } as any);

      const { result } = renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      expect(result.current.isStreaming).toBe(true);
    });
  });

  describe('Stream State Management', () => {
    it('initializes stream events when streaming starts', async () => {
      const mockEvents = createMockEventList(5);
      vi.mocked(useTaskEvents).mockReturnValue({
        data: mockEvents,
        isLoading: false,
        error: null,
      } as any);
      
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as any);

      renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      await waitFor(() => {
        expect(mockStoreActions.setStreamEvents).toHaveBeenCalledWith(mockTaskId, mockEvents);
      });
    });

    it('clears streaming state when streaming stops', async () => {
      const { rerender } = renderHook(
        ({ shouldStream }) => {
          // Mock dynamic streaming state
          vi.mocked(useTask).mockReturnValue({
            data: createMockTask({ is_running: shouldStream }),
            isLoading: false,
            error: null,
          } as any);
          
          return useTaskSSEConnection({ taskId: mockTaskId });
        },
        {
          wrapper: createWrapper(),
          initialProps: { shouldStream: true },
        }
      );

      // Verify streaming is active
      expect(mockStoreActions.setStreamEvents).toHaveBeenCalled();

      // Stop streaming
      rerender({ shouldStream: false });

      await waitFor(() => {
        expect(mockStoreActions.clearLocalMessages).toHaveBeenCalledWith(mockTaskId);
        expect(mockStoreActions.clearStreamEvents).toHaveBeenCalledWith(mockTaskId);
        expect(mockStoreActions.setStreamingError).toHaveBeenCalledWith(mockTaskId, null);
      });
    });

    it('cleans up when component unmounts', () => {
      const { unmount } = renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      unmount();

      expect(mockStoreActions.clearTaskStreaming).toHaveBeenCalledWith(mockTaskId);
    });
  });

  describe('SSE Integration', () => {
    it('handles SSE messages correctly', async () => {
      // Import the actual hook to test SSE callbacks
      const { useTaskSSE } = await import('../use-sse');
      const mockUseTaskSSE = vi.mocked(useTaskSSE);
      
      let capturedCallbacks: any = {};
      mockUseTaskSSE.mockImplementation((options) => {
        capturedCallbacks = {
          onMessage: options.onMessage,
          onEvent: options.onEvent,
          onError: options.onError,
        };
        return {
          isConnected: true,
          error: null,
          disconnect: vi.fn(),
        };
      });

      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as any);

      renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      // Test message handling
      const mockMessage = { id: 'msg-1', role: 'assistant', content: 'Test' };
      capturedCallbacks.onMessage(mockMessage);
      
      expect(mockStoreActions.addLocalMessage).toHaveBeenCalledWith(mockTaskId, mockMessage);
      expect(mockStoreActions.setStreamingError).toHaveBeenCalledWith(mockTaskId, null);

      // Test event handling
      const mockEvent = { event_type: 'message', data: {} };
      capturedCallbacks.onEvent(mockEvent);
      
      expect(mockStoreActions.addStreamEvent).toHaveBeenCalledWith(mockTaskId, mockEvent);

      // Test error handling
      capturedCallbacks.onError(new Error('Task not found'));
      expect(mockStoreActions.setStreamingError).toHaveBeenCalledWith(mockTaskId, 'Task not found');

      capturedCallbacks.onError(new Error('connection failed'));
      expect(mockStoreActions.setStreamingError).toHaveBeenCalledWith(
        mockTaskId, 
        'Connection failed after multiple attempts'
      );

      capturedCallbacks.onError(new Error('Unknown error'));
      expect(mockStoreActions.setStreamingError).toHaveBeenCalledWith(
        mockTaskId,
        'Streaming connection error'
      );
    });

    it('only connects SSE when streaming should be active', async () => {
      const { useTaskSSE } = await import('../use-sse');
      const mockUseTaskSSE = vi.mocked(useTaskSSE);
      
      mockUseTaskSSE.mockReturnValue({
        isConnected: false,
        error: null,
        disconnect: vi.fn(),
      });

      // Test with inactive state
      renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      expect(mockUseTaskSSE).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: mockTaskId,
          autoConnect: false, // Should not connect when inactive
        })
      );

      // Test with active state
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as any);

      renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      expect(mockUseTaskSSE).toHaveBeenLastCalledWith(
        expect.objectContaining({
          taskId: mockTaskId,
          autoConnect: true, // Should connect when active
        })
      );
    });
  });

  describe('Edge Cases', () => {
    it('handles missing task metadata gracefully', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
      } as any);

      const { result } = renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      expect(result.current.isStreaming).toBe(false);
    });

    it('handles missing workspace status gracefully', () => {
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
      } as any);

      const { result } = renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      expect(result.current.isStreaming).toBe(false);
    });

    it('does not initialize events when events data is missing', () => {
      vi.mocked(useTaskEvents).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
      } as any);
      
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as any);

      renderHook(() => useTaskSSEConnection({ taskId: mockTaskId }), {
        wrapper: createWrapper(),
      });

      expect(mockStoreActions.setStreamEvents).not.toHaveBeenCalled();
    });
  });
});