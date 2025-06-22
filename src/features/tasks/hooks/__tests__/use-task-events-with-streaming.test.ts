import { describe, expect, it, vi, beforeEach } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useTaskEventsWithStreaming } from '../use-task-events-with-streaming';
import { useTask, useTaskEvents, useTaskWorkspaceStatus } from '../use-tasks';
import { useTaskStreamEvents } from '../../stores/task-streaming-store';
import { createWrapper } from '@/test-utils/render';
import {
  createMockTask,
  createMockEventList,
  createMockWorkspaceStatus,
} from '@/test-utils/factories';

// Mock dependencies
vi.mock('../use-tasks');
vi.mock('../../stores/task-streaming-store');

describe('useTaskEventsWithStreaming', () => {
  const mockTaskId = 'task-123';
  const mockFetchedEvents = createMockEventList(3);
  const mockStreamEvents = createMockEventList(5);

  beforeEach(() => {
    vi.clearAllMocks();

    // Default mocks - task not running, workspace idle
    vi.mocked(useTask).mockReturnValue({
      data: createMockTask({
        id: mockTaskId,
        is_running: false,
        status: 'completed',
      }),
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: createMockWorkspaceStatus({ status: 'idle' }),
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskEvents).mockReturnValue({
      data: mockFetchedEvents,
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskStreamEvents).mockReturnValue([]);
  });

  describe('Streaming Logic', () => {
    it('uses fetched events when task and workspace are inactive', () => {
      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.events).toEqual(mockFetchedEvents);
      expect(result.current.isStreaming).toBe(false);
    });

    it('uses stream events when task is running', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as any);

      vi.mocked(useTaskStreamEvents).mockReturnValue(mockStreamEvents);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.events).toEqual(mockStreamEvents);
      expect(result.current.isStreaming).toBe(true);
    });

    it('uses stream events when task status is active', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: false, status: 'active' }),
        isLoading: false,
        error: null,
      } as any);

      vi.mocked(useTaskStreamEvents).mockReturnValue(mockStreamEvents);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.events).toEqual(mockStreamEvents);
      expect(result.current.isStreaming).toBe(true);
    });

    it('uses stream events when workspace is running', () => {
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: createMockWorkspaceStatus({ status: 'running' }),
        isLoading: false,
        error: null,
      } as any);

      vi.mocked(useTaskStreamEvents).mockReturnValue(mockStreamEvents);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.events).toEqual(mockStreamEvents);
      expect(result.current.isStreaming).toBe(true);
    });

    it('uses stream events when workspace is setting up', () => {
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: createMockWorkspaceStatus({ status: 'setting_up' }),
        isLoading: false,
        error: null,
      } as any);

      vi.mocked(useTaskStreamEvents).mockReturnValue(mockStreamEvents);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.events).toEqual(mockStreamEvents);
      expect(result.current.isStreaming).toBe(true);
    });

    it('falls back to fetched events when streaming but no stream events', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as any);

      // No stream events
      vi.mocked(useTaskStreamEvents).mockReturnValue([]);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.events).toEqual(mockFetchedEvents);
      expect(result.current.isStreaming).toBe(true);
    });

    it('uses stream events when either task OR workspace is active', () => {
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

      vi.mocked(useTaskStreamEvents).mockReturnValue(mockStreamEvents);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.events).toEqual(mockStreamEvents);
      expect(result.current.isStreaming).toBe(true);
    });
  });

  describe('Loading and Error States', () => {
    it('passes through loading state from useTaskEvents', () => {
      vi.mocked(useTaskEvents).mockReturnValue({
        data: undefined,
        isLoading: true,
        error: null,
      } as any);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.isLoading).toBe(true);
      expect(result.current.error).toBe(null);
      expect(result.current.events).toEqual([]);
    });

    it('passes through error state from useTaskEvents', () => {
      const mockError = new Error('Failed to fetch events');
      vi.mocked(useTaskEvents).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: mockError,
      } as any);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.error).toBe(mockError);
      expect(result.current.isLoading).toBe(false);
      expect(result.current.events).toEqual([]);
    });
  });

  describe('Edge Cases', () => {
    it('handles missing task metadata', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
      } as any);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.isStreaming).toBe(false);
      expect(result.current.events).toEqual(mockFetchedEvents);
    });

    it('handles missing workspace status', () => {
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
      } as any);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.isStreaming).toBe(false);
      expect(result.current.events).toEqual(mockFetchedEvents);
    });

    it('handles missing fetched events', () => {
      vi.mocked(useTaskEvents).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
      } as any);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.events).toEqual([]);
    });

    it('handles empty arrays correctly', () => {
      vi.mocked(useTaskEvents).mockReturnValue({
        data: [],
        isLoading: false,
        error: null,
      } as any);

      vi.mocked(useTaskStreamEvents).mockReturnValue([]);

      const { result } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.events).toEqual([]);
      expect(result.current.events).not.toBe(undefined);
    });
  });

  describe('Reactive Updates', () => {
    it('updates when task status changes', () => {
      const { result, rerender } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.isStreaming).toBe(false);

      // Update task to running
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as any);

      vi.mocked(useTaskStreamEvents).mockReturnValue(mockStreamEvents);

      rerender();

      expect(result.current.isStreaming).toBe(true);
      expect(result.current.events).toEqual(mockStreamEvents);
    });

    it('updates when workspace status changes', () => {
      const { result, rerender } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      expect(result.current.isStreaming).toBe(false);

      // Update workspace to running
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: createMockWorkspaceStatus({ status: 'running' }),
        isLoading: false,
        error: null,
      } as any);

      vi.mocked(useTaskStreamEvents).mockReturnValue(mockStreamEvents);

      rerender();

      expect(result.current.isStreaming).toBe(true);
      expect(result.current.events).toEqual(mockStreamEvents);
    });

    it('updates when stream events change', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as any);

      const { result, rerender } = renderHook(
        () => useTaskEventsWithStreaming(mockTaskId),
        { wrapper: createWrapper() },
      );

      // Initially no stream events
      expect(result.current.events).toEqual(mockFetchedEvents);

      // Add stream events
      vi.mocked(useTaskStreamEvents).mockReturnValue(mockStreamEvents);
      rerender();

      expect(result.current.events).toEqual(mockStreamEvents);
    });
  });
});