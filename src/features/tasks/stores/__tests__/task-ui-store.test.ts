import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';

import { createMockTaskIteration } from '@/test-utils/factories';

import {
  useRightPanelView,
  useSelectedIteration,
  useTaskUIStore,
} from '../task-ui-store';

describe('TaskUIStore', () => {
  beforeEach(() => {
    // Reset store to initial state
    act(() => {
      useTaskUIStore.getState().resetUIState();
    });
  });

  describe('Selected Iteration', () => {
    it('initializes with null selected iteration', () => {
      const state = useTaskUIStore.getState();
      expect(state.selectedIteration).toBe(null);
    });

    it('sets selected iteration', () => {
      const iteration = createMockTaskIteration({ iteration: 3 });

      act(() => {
        useTaskUIStore.getState().setSelectedIteration(iteration);
      });

      const state = useTaskUIStore.getState();
      expect(state.selectedIteration).toEqual(iteration);
    });

    it('clears selected iteration', () => {
      const iteration = createMockTaskIteration();

      act(() => {
        const store = useTaskUIStore.getState();
        store.setSelectedIteration(iteration);
        store.setSelectedIteration(null);
      });

      const state = useTaskUIStore.getState();
      expect(state.selectedIteration).toBe(null);
    });

    it('updates selected iteration', () => {
      const iteration1 = createMockTaskIteration({ iteration: 1 });
      const iteration2 = createMockTaskIteration({ iteration: 2 });

      act(() => {
        const store = useTaskUIStore.getState();
        store.setSelectedIteration(iteration1);
        store.setSelectedIteration(iteration2);
      });

      const state = useTaskUIStore.getState();
      expect(state.selectedIteration).toEqual(iteration2);
    });
  });

  describe('Right Panel View', () => {
    it('initializes with events view', () => {
      const state = useTaskUIStore.getState();
      expect(state.rightPanelView).toBe('events');
    });

    it('sets right panel view to diff', () => {
      act(() => {
        useTaskUIStore.getState().setRightPanelView('diff');
      });

      const state = useTaskUIStore.getState();
      expect(state.rightPanelView).toBe('diff');
    });

    it('toggles between views', () => {
      act(() => {
        const store = useTaskUIStore.getState();
        store.setRightPanelView('diff');
        store.setRightPanelView('events');
      });

      const state = useTaskUIStore.getState();
      expect(state.rightPanelView).toBe('events');
    });
  });

  describe('Reset UI State', () => {
    it('resets all state to initial values', () => {
      const iteration = createMockTaskIteration();

      act(() => {
        const store = useTaskUIStore.getState();
        store.setSelectedIteration(iteration);
        store.setRightPanelView('diff');
        store.resetUIState();
      });

      const state = useTaskUIStore.getState();
      expect(state.selectedIteration).toBe(null);
      expect(state.rightPanelView).toBe('events');
    });

    it('can be called multiple times safely', () => {
      act(() => {
        const store = useTaskUIStore.getState();
        store.resetUIState();
        store.resetUIState();
      });

      const state = useTaskUIStore.getState();
      expect(state.selectedIteration).toBe(null);
      expect(state.rightPanelView).toBe('events');
    });
  });

  describe('Selector Hooks', () => {
    it('useSelectedIteration returns current selected iteration', () => {
      const iteration = createMockTaskIteration({ iteration: 2 });

      const { result } = renderHook(() => useSelectedIteration());

      expect(result.current).toBe(null);

      act(() => {
        useTaskUIStore.getState().setSelectedIteration(iteration);
      });

      expect(result.current).toEqual(iteration);
    });

    it('useRightPanelView returns current panel view', () => {
      const { result } = renderHook(() => useRightPanelView());

      expect(result.current).toBe('events');

      act(() => {
        useTaskUIStore.getState().setRightPanelView('diff');
      });

      expect(result.current).toBe('diff');
    });

    it('selectors react to state changes', () => {
      const { result: iterationResult } = renderHook(() =>
        useSelectedIteration(),
      );
      const { result: viewResult } = renderHook(() => useRightPanelView());

      const iteration = createMockTaskIteration();

      act(() => {
        const store = useTaskUIStore.getState();
        store.setSelectedIteration(iteration);
        store.setRightPanelView('diff');
      });

      expect(iterationResult.current).toEqual(iteration);
      expect(viewResult.current).toBe('diff');
    });
  });

  describe('State Persistence', () => {
    it('maintains state across multiple getState calls', () => {
      const iteration = createMockTaskIteration();

      act(() => {
        useTaskUIStore.getState().setSelectedIteration(iteration);
      });

      const state1 = useTaskUIStore.getState();
      const state2 = useTaskUIStore.getState();

      expect(state1.selectedIteration).toBe(state2.selectedIteration);
    });

    it('state changes are immediately reflected', () => {
      const iteration1 = createMockTaskIteration({ iteration: 1 });
      const iteration2 = createMockTaskIteration({ iteration: 2 });

      act(() => {
        useTaskUIStore.getState().setSelectedIteration(iteration1);
      });

      expect(useTaskUIStore.getState().selectedIteration).toEqual(iteration1);

      act(() => {
        useTaskUIStore.getState().setSelectedIteration(iteration2);
      });

      expect(useTaskUIStore.getState().selectedIteration).toEqual(iteration2);
    });
  });

  describe('Subscriptions', () => {
    it('notifies subscribers on state change', () => {
      let notificationCount = 0;

      const unsubscribe = useTaskUIStore.subscribe(() => {
        notificationCount++;
      });

      act(() => {
        useTaskUIStore.getState().setRightPanelView('diff');
      });

      expect(notificationCount).toBe(1);

      act(() => {
        useTaskUIStore
          .getState()
          .setSelectedIteration(createMockTaskIteration());
      });

      expect(notificationCount).toBe(2);

      unsubscribe();
    });

    it('stops notifying after unsubscribe', () => {
      let notificationCount = 0;

      const unsubscribe = useTaskUIStore.subscribe(() => {
        notificationCount++;
      });

      act(() => {
        useTaskUIStore.getState().setRightPanelView('diff');
      });

      expect(notificationCount).toBe(1);

      unsubscribe();

      act(() => {
        useTaskUIStore.getState().setRightPanelView('events');
      });

      expect(notificationCount).toBe(1); // Should not increase
    });
  });

  describe('Type Safety', () => {
    it('only accepts valid panel views', () => {
      const store = useTaskUIStore.getState();

      // TypeScript should enforce this at compile time
      // but we can test runtime behavior
      act(() => {
        store.setRightPanelView('events');
        store.setRightPanelView('diff');
      });

      expect(['events', 'diff']).toContain(store.rightPanelView);
    });

    it('handles iteration object correctly', () => {
      const iteration = createMockTaskIteration({
        iteration: 1,
        status: 'completed',
        initial_message: 'Test',
      });

      act(() => {
        useTaskUIStore.getState().setSelectedIteration(iteration);
      });

      const state = useTaskUIStore.getState();
      expect(state.selectedIteration?.iteration_id).toBe('iter-1');
      expect(state.selectedIteration?.status).toBe('completed');
      expect(state.selectedIteration?.initial_message).toBe('Test');
    });
  });
});
