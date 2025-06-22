import { useMemo } from 'react';

import {
  useTask,
  useTaskEvents,
  useTaskWorkspaceStatus,
} from '@/features/tasks/hooks/use-tasks';
import { useTaskStreamEvents } from '@/features/tasks/stores/task-streaming-store';
import type { Event } from '@/types/event';

interface UseTaskEventsWithStreamingResult {
  events: Array<Event>;
  isLoading: boolean;
  error: Error | null;
  isStreaming: boolean;
}

/**
 * Hook that provides events with automatic streaming support.
 * - When task or workspace is running/setting up, returns stream events
 * - Otherwise returns regular fetched events
 * - Handles all the logic for determining when to stream vs fetch
 */
export function useTaskEventsWithStreaming(
  taskId: string,
): UseTaskEventsWithStreamingResult {
  // Get task metadata to check if it's running
  const { data: metadata } = useTask(taskId);

  // Get workspace status to check if it's setting up or running
  const { data: workspaceStatus } = useTaskWorkspaceStatus(taskId);

  // Get regular events from API
  const { data: fetchedEvents, isLoading, error } = useTaskEvents(taskId);

  // Get stream events from Zustand store
  const streamEvents = useTaskStreamEvents(taskId);

  // Determine if we should use streaming
  const isStreaming = useMemo(() => {
    const taskRunning = metadata?.is_running || metadata?.status === 'active';
    const workspaceActive =
      workspaceStatus?.status === 'running' ||
      workspaceStatus?.status === 'setting_up';

    return taskRunning || workspaceActive;
  }, [metadata, workspaceStatus]);

  // Choose which events to display
  const events = useMemo(() => {
    if (isStreaming && streamEvents.length > 0) {
      return streamEvents;
    }
    return fetchedEvents || [];
  }, [isStreaming, streamEvents, fetchedEvents]);

  return {
    events,
    isLoading,
    error,
    isStreaming,
  };
}
