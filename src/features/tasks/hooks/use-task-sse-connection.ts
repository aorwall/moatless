import { useCallback, useEffect, useMemo } from 'react';

import { useTaskSSE } from '@/features/tasks/hooks/use-sse';
import {
  useTask,
  useTaskEvents,
  useTaskWorkspaceStatus,
} from '@/features/tasks/hooks/use-tasks';
import { useTaskStreamingStore } from '@/features/tasks/stores/task-streaming-store';
import type { Event } from '@/types/event';

interface UseTaskSSEConnectionOptions {
  taskId: string;
}

export function useTaskSSEConnection({ taskId }: UseTaskSSEConnectionOptions) {
  const { data: metadata } = useTask(taskId);
  const { data: events } = useTaskEvents(taskId);
  const { data: workspaceStatus } = useTaskWorkspaceStatus(taskId);

  // Get streaming actions from Zustand
  const {
    addStreamEvent,
    setStreamEvents,
    setStreamingError,
    clearStreamEvents,
    clearTaskStreaming,
  } = useTaskStreamingStore();

  // Determine if we should stream based on task or workspace status
  const shouldStream = useMemo(() => {
    const taskRunning = metadata?.is_running || metadata?.status === 'active';
    const workspaceActive =
      workspaceStatus?.status === 'running' ||
      workspaceStatus?.status === 'setting_up';

    return taskRunning || workspaceActive;
  }, [metadata, workspaceStatus]);

  // Initialize stream events with existing events when streaming starts
  useEffect(() => {
    if (shouldStream && events) {
      setStreamEvents(taskId, events);
    }
  }, [shouldStream, events, setStreamEvents, taskId]);

  // Clear streaming state when streaming should stop
  useEffect(() => {
    if (!shouldStream) {
      clearStreamEvents(taskId);
      setStreamingError(taskId, null);
    }
  }, [shouldStream, clearStreamEvents, setStreamingError, taskId]);

  // Clean up when component unmounts
  useEffect(() => {
    return () => {
      clearTaskStreaming(taskId);
    };
  }, [taskId, clearTaskStreaming]);

  // Memoize SSE callbacks to prevent reconnections
  const handleSSEEvent = useCallback(
    (newEvent: Event) => {
      addStreamEvent(taskId, newEvent);
      setStreamingError(taskId, null);
    },
    [addStreamEvent, setStreamingError, taskId],
  );

  const handleSSEError = useCallback(
    (error: Error) => {
      console.error('SSE error:', error);
      if (error.message.includes('Task not found')) {
        setStreamingError(taskId, 'Task not found');
      } else if (error.message.includes('connection failed')) {
        setStreamingError(taskId, 'Connection failed after multiple attempts');
      } else {
        setStreamingError(taskId, 'Streaming connection error');
      }
    },
    [setStreamingError, taskId],
  );

  // Use SSE for real-time updates only if streaming should be active
  useTaskSSE({
    taskId,
    autoConnect: shouldStream,
    onEvent: handleSSEEvent,
    onError: handleSSEError,
  });

  return {
    isStreaming: shouldStream,
  };
}
