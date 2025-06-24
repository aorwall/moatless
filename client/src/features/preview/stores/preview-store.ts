import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';
import type { ElementInfo, PreviewConfig } from '../types';

interface PreviewStore {
  // Configuration
  config: PreviewConfig;
  currentPath: string;

  // UI State
  isInspectorActive: boolean;
  isLoaded: boolean;

  // Data
  selectedElement: ElementInfo | null;
  error: string | null;
  isConnected: boolean;

  // Actions
  setConfig: (config: PreviewConfig) => void;
  setCurrentPath: (path: string) => void;
  toggleInspector: () => void;
  setSelectedElement: (element: ElementInfo | null) => void;
  setError: (error: string | null) => void;
  setConnected: (connected: boolean) => void;
  setLoaded: (loaded: boolean) => void;
  reset: () => void;
}

const initialState = {
  config: { url: 'http://localhost:3000' },
  currentPath: '/',
  isInspectorActive: false,
  isLoaded: false,
  selectedElement: null,
  error: null,
  isConnected: false,
};

export const usePreviewStore = create<PreviewStore>()(
  immer((set) => ({
    ...initialState,

    setConfig: (config) =>
      set((state) => {
        state.config = config;
      }),

    setCurrentPath: (path) =>
      set((state) => {
        state.currentPath = path.startsWith('/') ? path : `/${path}`;
      }),

    toggleInspector: () =>
      set((state) => {
        state.isInspectorActive = !state.isInspectorActive;
        if (!state.isInspectorActive) {
          state.selectedElement = null;
        }
      }),

    setSelectedElement: (element) =>
      set((state) => {
        state.selectedElement = element;
      }),

    setError: (error) =>
      set((state) => {
        state.error = error;
        if (error) {
          state.isLoaded = false;
        }
      }),

    setConnected: (connected) =>
      set((state) => {
        state.isConnected = connected;
      }),

    setLoaded: (loaded) =>
      set((state) => {
        state.isLoaded = loaded;
        if (loaded) {
          state.error = null;
        }
      }),

    reset: () => set(() => ({ ...initialState })),
  }))
);