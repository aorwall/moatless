import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, waitFor, act } from '@testing-library/react';
import { useTaskSSE } from '../use-sse';
import { TaskSSEClient } from '@/features/tasks/api/sse';
import { createWrapper } from '@/test-utils/render';
import { createMockClaudeMessage, createMockEvent } from '@/test-utils/factories';

// Mock TaskSSEClient
vi.mock('@/features/tasks/api/sse', () => ({
  TaskSSEClient: vi.fn(),
}));

describe('useTaskSSE', () => {
  let mockSSEClient: any;
  const mockTaskId = 'task-123';

  beforeEach(() => {
    vi.clearAllMocks();
    
    // Create mock SSE client instance
    mockSSEClient = {
      connect: vi.fn(),
      disconnect: vi.fn(),
      isConnected: false,
    };

    // Mock the constructor
    vi.mocked(TaskSSEClient).mockImplementation(() => mockSSEClient);
  });

  afterEach(() => {
    vi.clearAllTimers();
  });

  describe('Connection Management', () => {
    it('creates SSE client with correct options', () => {
      const onMessage = vi.fn();
      const onEvent = vi.fn();
      const onError = vi.fn();
      const apiUrl = 'http://test.api';

      renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
            apiUrl,
            onMessage,
            onEvent,
            onError,
          }),
        { wrapper: createWrapper() },
      );

      expect(TaskSSEClient).toHaveBeenCalledWith({
        taskId: mockTaskId,
        apiUrl,
        onMessage: expect.any(Function),
        onEvent: expect.any(Function),
        onError: expect.any(Function),
        onConnect: expect.any(Function),
        onDisconnect: expect.any(Function),
        onTaskNotFound: expect.any(Function),
      });
    });

    it('auto-connects when autoConnect is true', () => {
      renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
            autoConnect: true,
          }),
        { wrapper: createWrapper() },
      );

      expect(mockSSEClient.connect).toHaveBeenCalled();
    });

    it('does not auto-connect when autoConnect is false', () => {
      renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
            autoConnect: false,
          }),
        { wrapper: createWrapper() },
      );

      expect(mockSSEClient.connect).not.toHaveBeenCalled();
    });

    it('connects manually when connect is called', () => {
      const { result } = renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
            autoConnect: false,
          }),
        { wrapper: createWrapper() },
      );

      expect(mockSSEClient.connect).not.toHaveBeenCalled();

      act(() => {
        result.current.connect();
      });

      expect(mockSSEClient.connect).toHaveBeenCalled();
    });

    it('disconnects when disconnect is called', () => {
      const { result } = renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
          }),
        { wrapper: createWrapper() },
      );

      act(() => {
        result.current.disconnect();
      });

      expect(mockSSEClient.disconnect).toHaveBeenCalled();
    });

    it('disconnects on unmount', () => {
      const { unmount } = renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
          }),
        { wrapper: createWrapper() },
      );

      unmount();

      expect(mockSSEClient.disconnect).toHaveBeenCalled();
    });

    it('recreates client when taskId changes', () => {
      const { rerender } = renderHook(
        ({ taskId }) =>
          useTaskSSE({
            taskId,
          }),
        {
          wrapper: createWrapper(),
          initialProps: { taskId: 'task-1' },
        },
      );

      expect(TaskSSEClient).toHaveBeenCalledTimes(1);
      expect(mockSSEClient.disconnect).not.toHaveBeenCalled();

      rerender({ taskId: 'task-2' });

      expect(mockSSEClient.disconnect).toHaveBeenCalled();
      expect(TaskSSEClient).toHaveBeenCalledTimes(2);
    });

    it('handles autoConnect changes', () => {
      const { rerender } = renderHook(
        ({ autoConnect }) =>
          useTaskSSE({
            taskId: mockTaskId,
            autoConnect,
          }),
        {
          wrapper: createWrapper(),
          initialProps: { autoConnect: false },
        },
      );

      expect(mockSSEClient.connect).not.toHaveBeenCalled();

      rerender({ autoConnect: true });
      expect(mockSSEClient.connect).toHaveBeenCalled();

      rerender({ autoConnect: false });
      expect(mockSSEClient.disconnect).toHaveBeenCalled();
    });
  });

  describe('Event Handling', () => {
    it('handles onConnect callback', () => {
      let capturedCallbacks: any = {};
      vi.mocked(TaskSSEClient).mockImplementation((options) => {
        capturedCallbacks = options;
        return mockSSEClient;
      });

      const { result } = renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
          }),
        { wrapper: createWrapper() },
      );

      expect(result.current.isConnected).toBe(false);

      act(() => {
        capturedCallbacks.onConnect();
      });

      expect(result.current.isConnected).toBe(true);
      expect(result.current.error).toBe(null);
    });

    it('handles onDisconnect callback', () => {
      let capturedCallbacks: any = {};
      vi.mocked(TaskSSEClient).mockImplementation((options) => {
        capturedCallbacks = options;
        return mockSSEClient;
      });

      const { result } = renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
          }),
        { wrapper: createWrapper() },
      );

      act(() => {
        capturedCallbacks.onConnect();
      });
      expect(result.current.isConnected).toBe(true);

      act(() => {
        capturedCallbacks.onDisconnect();
      });
      expect(result.current.isConnected).toBe(false);
    });

    it('handles onMessage callback', () => {
      let capturedCallbacks: any = {};
      vi.mocked(TaskSSEClient).mockImplementation((options) => {
        capturedCallbacks = options;
        return mockSSEClient;
      });

      const onMessage = vi.fn();
      const { result } = renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
            onMessage,
          }),
        { wrapper: createWrapper() },
      );

      const mockMessage = createMockClaudeMessage({ content: 'Test message' });

      act(() => {
        capturedCallbacks.onMessage(mockMessage);
      });

      expect(result.current.lastMessage).toEqual(mockMessage);
      expect(result.current.error).toBe(null);
      expect(onMessage).toHaveBeenCalledWith(mockMessage);
    });

    it('handles onEvent callback', () => {
      let capturedCallbacks: any = {};
      vi.mocked(TaskSSEClient).mockImplementation((options) => {
        capturedCallbacks = options;
        return mockSSEClient;
      });

      const onEvent = vi.fn();
      renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
            onEvent,
          }),
        { wrapper: createWrapper() },
      );

      const mockEvent = createMockEvent({ event_type: 'message' });

      act(() => {
        capturedCallbacks.onEvent(mockEvent);
      });

      expect(onEvent).toHaveBeenCalledWith(mockEvent);
    });

    it('handles onError callback', () => {
      let capturedCallbacks: any = {};
      vi.mocked(TaskSSEClient).mockImplementation((options) => {
        capturedCallbacks = options;
        return mockSSEClient;
      });

      const onError = vi.fn();
      const { result } = renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
            onError,
          }),
        { wrapper: createWrapper() },
      );

      const mockError = new Error('Connection failed');

      act(() => {
        capturedCallbacks.onError(mockError);
      });

      expect(result.current.error).toEqual(mockError);
      expect(onError).toHaveBeenCalledWith(mockError);
    });

    it('handles onTaskNotFound callback', () => {
      let capturedCallbacks: any = {};
      vi.mocked(TaskSSEClient).mockImplementation((options) => {
        capturedCallbacks = options;
        return mockSSEClient;
      });

      const { result } = renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
          }),
        { wrapper: createWrapper() },
      );

      act(() => {
        capturedCallbacks.onTaskNotFound();
      });

      expect(result.current.error?.message).toBe('Task not found');
      expect(result.current.isConnected).toBe(false);
    });
  });

  describe('Callback Stability', () => {
    it('maintains stable callbacks when props change', () => {
      let capturedCallbacks: any = {};
      let callCount = 0;
      
      vi.mocked(TaskSSEClient).mockImplementation((options) => {
        capturedCallbacks = options;
        callCount++;
        return mockSSEClient;
      });

      const { rerender } = renderHook(
        ({ onMessage }) =>
          useTaskSSE({
            taskId: mockTaskId,
            onMessage,
          }),
        {
          wrapper: createWrapper(),
          initialProps: { onMessage: vi.fn() },
        },
      );

      const initialCallbacks = { ...capturedCallbacks };
      
      // Change the onMessage prop
      rerender({ onMessage: vi.fn() });

      // TaskSSEClient should not be recreated
      expect(callCount).toBe(1);
      
      // Callbacks should remain the same reference
      expect(capturedCallbacks.onMessage).toBe(initialCallbacks.onMessage);
      expect(capturedCallbacks.onEvent).toBe(initialCallbacks.onEvent);
      expect(capturedCallbacks.onError).toBe(initialCallbacks.onError);
    });

    it('updates callback behavior without recreating client', () => {
      let capturedCallbacks: any = {};
      vi.mocked(TaskSSEClient).mockImplementation((options) => {
        capturedCallbacks = options;
        return mockSSEClient;
      });

      const firstOnMessage = vi.fn();
      const secondOnMessage = vi.fn();

      const { rerender } = renderHook(
        ({ onMessage }) =>
          useTaskSSE({
            taskId: mockTaskId,
            onMessage,
          }),
        {
          wrapper: createWrapper(),
          initialProps: { onMessage: firstOnMessage },
        },
      );

      const mockMessage = createMockClaudeMessage();

      // Call with first callback
      act(() => {
        capturedCallbacks.onMessage(mockMessage);
      });
      expect(firstOnMessage).toHaveBeenCalledWith(mockMessage);
      expect(secondOnMessage).not.toHaveBeenCalled();

      // Update callback
      rerender({ onMessage: secondOnMessage });

      // Call with second callback
      act(() => {
        capturedCallbacks.onMessage(mockMessage);
      });
      expect(secondOnMessage).toHaveBeenCalledWith(mockMessage);
      expect(firstOnMessage).toHaveBeenCalledTimes(1); // Not called again
    });
  });

  describe('Error Handling', () => {
    it('clears error when message is received', () => {
      let capturedCallbacks: any = {};
      vi.mocked(TaskSSEClient).mockImplementation((options) => {
        capturedCallbacks = options;
        return mockSSEClient;
      });

      const { result } = renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
          }),
        { wrapper: createWrapper() },
      );

      // Set an error
      act(() => {
        capturedCallbacks.onError(new Error('Test error'));
      });
      expect(result.current.error).not.toBe(null);

      // Receive a message
      act(() => {
        capturedCallbacks.onMessage(createMockClaudeMessage());
      });
      expect(result.current.error).toBe(null);
    });

    it('clears error when event is received', () => {
      let capturedCallbacks: any = {};
      vi.mocked(TaskSSEClient).mockImplementation((options) => {
        capturedCallbacks = options;
        return mockSSEClient;
      });

      const { result } = renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
          }),
        { wrapper: createWrapper() },
      );

      // Set an error
      act(() => {
        capturedCallbacks.onError(new Error('Test error'));
      });
      expect(result.current.error).not.toBe(null);

      // Receive an event
      act(() => {
        capturedCallbacks.onEvent(createMockEvent());
      });
      expect(result.current.error).toBe(null);
    });

    it('clears error on connect', () => {
      let capturedCallbacks: any = {};
      vi.mocked(TaskSSEClient).mockImplementation((options) => {
        capturedCallbacks = options;
        return mockSSEClient;
      });

      const { result } = renderHook(
        () =>
          useTaskSSE({
            taskId: mockTaskId,
          }),
        { wrapper: createWrapper() },
      );

      // Set an error
      act(() => {
        capturedCallbacks.onError(new Error('Test error'));
      });
      expect(result.current.error).not.toBe(null);

      // Connect
      act(() => {
        capturedCallbacks.onConnect();
      });
      expect(result.current.error).toBe(null);
    });
  });
});