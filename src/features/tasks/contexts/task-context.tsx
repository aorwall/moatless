import { useParams } from '@tanstack/react-router';
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from 'react';

import { useTaskSSE } from '@/features/tasks/hooks/use-sse';
import {
  useTask,
  useTaskEvents,
  useTaskGitDiff,
  useTaskWorkspaceStatus,
} from '@/features/tasks/hooks/use-tasks';
import type { Event } from '@/types/event';
import type { GitDiffResponse } from '@/types/git-diff';
import type {
  BaseMessage,
  ClaudeMessage,
  TaskIteration,
  TaskMetadata,
  WorkspaceStatusResponse,
} from '@/types/task';

interface TaskContextValue {
  // Task ID
  taskId: string;

  // Task metadata
  metadata: TaskMetadata | undefined;
  isLoadingMetadata: boolean;
  metadataError: Error | null;

  // Task events
  events: Array<Event> | undefined;
  streamEvents: Array<Event>;
  isLoadingEvents: boolean;
  eventsError: Error | null;

  // Git diff
  gitDiff: GitDiffResponse | undefined | null;
  isLoadingGitDiff: boolean;
  gitDiffError: Error | null;

  // Workspace status
  workspaceStatus: WorkspaceStatusResponse | undefined | null;
  isLoadingWorkspace: boolean;
  workspaceError: Error | null;

  // Task state
  isRunning: boolean;
  taskStatus: string | undefined;

  // SSE state
  streamingError: string | null;
  localMessages: Array<ClaudeMessage>;

  // UI state
  selectedIteration: TaskIteration | null;
  setSelectedIteration: (iteration: TaskIteration | null) => void;
  rightPanelView: 'events' | 'diff';
  setRightPanelView: (view: 'events' | 'diff') => void;
}

export const TaskContext = createContext<TaskContextValue | undefined>(
  undefined,
);

export function useTaskContext() {
  const context = useContext(TaskContext);
  if (!context) {
    throw new Error('useTaskContext must be used within a TaskProvider');
  }
  return context;
}

interface TaskProviderProps {
  children: ReactNode;
}

export function TaskProvider({ children }: TaskProviderProps) {
  const params = useParams({ from: '/tasks/$taskId' });
  const taskId = params.taskId;

  // Local state
  const [localMessages, setLocalMessages] = useState<Array<ClaudeMessage>>([]);
  const [streamingError, setStreamingError] = useState<string | null>(null);
  const [selectedIteration, setSelectedIteration] =
    useState<TaskIteration | null>(null);
  const [rightPanelView, setRightPanelView] = useState<'events' | 'diff'>(
    'events',
  );
  const [streamEvents, setStreamEvents] = useState<Array<Event>>([]);

  // Use React Query hooks
  const {
    data: metadata,
    isLoading: isLoadingMetadata,
    error: metadataError,
  } = useTask(taskId);

  const {
    data: events,
    isLoading: isLoadingEvents,
    error: eventsError,
  } = useTaskEvents(taskId);

  const {
    data: gitDiff,
    isLoading: isLoadingGitDiff,
    error: gitDiffError,
  } = useTaskGitDiff(taskId);

  const {
    data: workspaceStatus,
    isLoading: isLoadingWorkspace,
    error: workspaceError,
  } = useTaskWorkspaceStatus(taskId);

  // Extract task status as a stable value
  const taskStatus = metadata?.status;

  // Determine if the task is in a running state
  const isRunning = useMemo(() => {
    if (!metadata) {
      return false;
    }

    // Task is running if:
    // 1. Task metadata says it's running OR
    // 2. Task is active AND workspace is setting up or running
    const workspaceRunning =
      workspaceStatus?.status === 'setting_up' ||
      workspaceStatus?.status === 'running';

    return metadata.is_running || (taskStatus === 'active' && workspaceRunning);
  }, [metadata, taskStatus, workspaceStatus]);

  // Initialize stream events with existing events when task is running
  useEffect(() => {
    if (isRunning && events && streamEvents.length === 0) {
      setStreamEvents(events);
    }
  }, [isRunning, events, streamEvents.length]);

  // Memoize SSE callbacks to prevent reconnections
  const handleSSEMessage = useCallback((newMessage: BaseMessage) => {
    // Add message to local state for immediate UI update
    setLocalMessages((prev) => [...prev, newMessage as ClaudeMessage]);
    setStreamingError(null); // Clear error on successful message
  }, []);

  const handleSSEEvent = useCallback((newEvent: Event) => {
    // Add event to stream events
    setStreamEvents((prev) => [...prev, newEvent]);
    setStreamingError(null); // Clear error on successful event
  }, []);

  const handleSSEError = useCallback((error: Error) => {
    console.error('SSE error:', error);
    // If task not found, disable streaming
    if (error.message.includes('Task not found')) {
      setStreamingError('Task not found');
    } else if (error.message.includes('connection failed')) {
      setStreamingError('Connection failed after multiple attempts');
    } else {
      setStreamingError('Streaming connection error');
    }
  }, []);

  // Memoize autoConnect to prevent unnecessary effect triggers
  const autoConnect = useMemo(() => {
    return isRunning;
  }, [isRunning]);

  // Use SSE for real-time updates only if task exists and streaming is enabled
  useTaskSSE({
    taskId,
    autoConnect,
    onMessage: handleSSEMessage,
    onEvent: handleSSEEvent,
    onError: handleSSEError,
  });

  const contextValue: TaskContextValue = {
    // Task ID
    taskId,

    // Task metadata
    metadata,
    isLoadingMetadata,
    metadataError: metadataError as Error | null,

    // Task events
    events,
    streamEvents,
    isLoadingEvents,
    eventsError: eventsError as Error | null,

    // Git diff
    gitDiff,
    isLoadingGitDiff,
    gitDiffError: gitDiffError as Error | null,

    // Workspace status
    workspaceStatus,
    isLoadingWorkspace,
    workspaceError: workspaceError as Error | null,

    // Task state
    isRunning,
    taskStatus,

    // SSE state
    streamingError,
    localMessages,

    // UI state
    selectedIteration,
    setSelectedIteration,
    rightPanelView,
    setRightPanelView,
  };

  return (
    <TaskContext.Provider value={contextValue}>{children}</TaskContext.Provider>
  );
}
