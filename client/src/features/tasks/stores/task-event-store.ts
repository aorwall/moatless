import { create } from 'zustand';
import { subscribeWithSelector } from 'zustand/middleware';

import type { Event } from '@/types/event';

interface TaskEventState {
  // Event state per task
  eventTasks: Map<
    string,
    {
      streamEvents: Array<Event>;
      lastEventId: string | null;
      streamingError: string | null;
      isStreaming: boolean;
    }
  >;

  // Actions
  addStreamEvent: (taskId: string, event: Event) => void;
  setStreamEvents: (taskId: string, events: Array<Event>) => void;
  clearStreamEvents: (taskId: string) => void;
  setLastEventId: (taskId: string, eventId: string | null) => void;
  setStreamingError: (taskId: string, error: string | null) => void;
  setIsStreaming: (taskId: string, streaming: boolean) => void;
  clearTaskEvents: (taskId: string) => void;

  // Selectors
  getTaskEventState: (taskId: string) => {
    streamEvents: Array<Event>;
    lastEventId: string | null;
    streamingError: string | null;
    isStreaming: boolean;
  };
}

const getDefaultTaskState = () => ({
  streamEvents: [],
  lastEventId: null,
  streamingError: null,
  isStreaming: false,
});

export const useTaskEventStore = create<TaskEventState>()(
  subscribeWithSelector((set, get) => ({
    eventTasks: new Map(),

    // Actions
    addStreamEvent: (taskId, event) =>
      set((state) => {
        const newTasks = new Map(state.eventTasks);
        const taskState = newTasks.get(taskId) || getDefaultTaskState();
        const updatedEvents = [...taskState.streamEvents, event];
        const lastEventId = event.event_id || taskState.lastEventId;
        newTasks.set(taskId, {
          ...taskState,
          streamEvents: updatedEvents,
          lastEventId,
        });
        return { eventTasks: newTasks };
      }),

    setStreamEvents: (taskId, events) =>
      set((state) => {
        const newTasks = new Map(state.eventTasks);
        const taskState = newTasks.get(taskId) || getDefaultTaskState();
        const lastEventId =
          events.length > 0
            ? events[events.length - 1].event_id || taskState.lastEventId
            : taskState.lastEventId;
        newTasks.set(taskId, {
          ...taskState,
          streamEvents: events,
          lastEventId,
        });
        return { eventTasks: newTasks };
      }),

    clearStreamEvents: (taskId) =>
      set((state) => {
        const newTasks = new Map(state.eventTasks);
        const taskState = newTasks.get(taskId);
        if (taskState) {
          newTasks.set(taskId, {
            ...taskState,
            streamEvents: [],
          });
        }
        return { eventTasks: newTasks };
      }),

    setLastEventId: (taskId, eventId) =>
      set((state) => {
        const newTasks = new Map(state.eventTasks);
        const taskState = newTasks.get(taskId) || getDefaultTaskState();
        newTasks.set(taskId, {
          ...taskState,
          lastEventId: eventId,
        });
        return { eventTasks: newTasks };
      }),

    setStreamingError: (taskId, error) =>
      set((state) => {
        const newTasks = new Map(state.eventTasks);
        const taskState = newTasks.get(taskId) || getDefaultTaskState();
        newTasks.set(taskId, {
          ...taskState,
          streamingError: error,
        });
        return { eventTasks: newTasks };
      }),

    setIsStreaming: (taskId, streaming) =>
      set((state) => {
        const newTasks = new Map(state.eventTasks);
        const taskState = newTasks.get(taskId) || getDefaultTaskState();
        newTasks.set(taskId, {
          ...taskState,
          isStreaming: streaming,
        });
        return { eventTasks: newTasks };
      }),

    clearTaskEvents: (taskId) =>
      set((state) => {
        const newTasks = new Map(state.eventTasks);
        newTasks.delete(taskId);
        return { eventTasks: newTasks };
      }),

    // Selectors
    getTaskEventState: (taskId) => {
      const state = get().eventTasks.get(taskId);
      return state || getDefaultTaskState();
    },
  })),
);

// Convenience hooks with stable selectors
export const useTaskStreamEvents = (taskId: string) =>
  useTaskEventStore((state) => {
    const taskState = state.eventTasks.get(taskId);
    return taskState ? taskState.streamEvents : [];
  });

export const useTaskStreamingError = (taskId: string) =>
  useTaskEventStore((state) => {
    const taskState = state.eventTasks.get(taskId);
    return taskState ? taskState.streamingError : null;
  });

export const useTaskLastEventId = (taskId: string) =>
  useTaskEventStore((state) => {
    const taskState = state.eventTasks.get(taskId);
    return taskState ? taskState.lastEventId : null;
  });

export const useTaskIsStreaming = (taskId: string) =>
  useTaskEventStore((state) => {
    const taskState = state.eventTasks.get(taskId);
    return taskState ? taskState.isStreaming : false;
  });
