import { create } from 'zustand';
import { subscribeWithSelector } from 'zustand/middleware';

import type { Event } from '@/types/event';

interface TaskStreamingState {
  // Streaming state per task
  streamingTasks: Map<
    string,
    {
      streamEvents: Array<Event>;
      streamingError: string | null;
    }
  >;

  // Actions
  addStreamEvent: (taskId: string, event: Event) => void;
  setStreamEvents: (taskId: string, events: Array<Event>) => void;
  clearStreamEvents: (taskId: string) => void;
  setStreamingError: (taskId: string, error: string | null) => void;
  clearTaskStreaming: (taskId: string) => void;

  // Selectors
  getTaskStreamingState: (taskId: string) => {
    streamEvents: Array<Event>;
    streamingError: string | null;
  };
}

const getDefaultTaskState = () => ({
  streamEvents: [],
  streamingError: null,
});

export const useTaskStreamingStore = create<TaskStreamingState>()(
  subscribeWithSelector((set, get) => ({
    streamingTasks: new Map(),

    // Actions
    addStreamEvent: (taskId, event) =>
      set((state) => {
        const newTasks = new Map(state.streamingTasks);
        const taskState = newTasks.get(taskId) || getDefaultTaskState();
        newTasks.set(taskId, {
          ...taskState,
          streamEvents: [...taskState.streamEvents, event],
        });
        return { streamingTasks: newTasks };
      }),

    setStreamEvents: (taskId, events) =>
      set((state) => {
        const newTasks = new Map(state.streamingTasks);
        const taskState = newTasks.get(taskId) || getDefaultTaskState();
        newTasks.set(taskId, {
          ...taskState,
          streamEvents: events,
        });
        return { streamingTasks: newTasks };
      }),

    clearStreamEvents: (taskId) =>
      set((state) => {
        const newTasks = new Map(state.streamingTasks);
        const taskState = newTasks.get(taskId);
        if (taskState) {
          newTasks.set(taskId, {
            ...taskState,
            streamEvents: [],
          });
        }
        return { streamingTasks: newTasks };
      }),

    setStreamingError: (taskId, error) =>
      set((state) => {
        const newTasks = new Map(state.streamingTasks);
        const taskState = newTasks.get(taskId) || getDefaultTaskState();
        newTasks.set(taskId, {
          ...taskState,
          streamingError: error,
        });
        return { streamingTasks: newTasks };
      }),

    clearTaskStreaming: (taskId) =>
      set((state) => {
        const newTasks = new Map(state.streamingTasks);
        newTasks.delete(taskId);
        return { streamingTasks: newTasks };
      }),

    // Selectors
    getTaskStreamingState: (taskId) => {
      const state = get().streamingTasks.get(taskId);
      return state || getDefaultTaskState();
    },
  })),
);

// Convenience hooks with stable selectors
export const useTaskStreamEvents = (taskId: string) =>
  useTaskStreamingStore((state) => {
    const taskState = state.streamingTasks.get(taskId);
    return taskState ? taskState.streamEvents : [];
  });

export const useTaskStreamingError = (taskId: string) =>
  useTaskStreamingStore((state) => {
    const taskState = state.streamingTasks.get(taskId);
    return taskState ? taskState.streamingError : null;
  });
