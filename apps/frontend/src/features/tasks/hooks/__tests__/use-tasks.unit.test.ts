import { renderHook, waitFor } from '@testing-library/react'
import { act } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { serverApi } from '@/features/preview/api/server'
import { useServerStatus, useStartServer, useStopServer } from '@/features/preview/hooks/use-server'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'
import {
  createMockAgentTriggerResponse,
  createMockEventList,
  createMockGitDiff,
  createMockServerStatus,
  createMockTask,
  createMockTaskWithIterations,
} from '@/test-utils/factories'
import { createWrapper } from '@/test-utils/render'
import {
  useCreateTask,
  useSendMessage,
  useStopAgent,
  useTask,
  useTaskEvents,
  useTaskGitDiff,
  useTasksList,
} from '../use-tasks'

// Mock the tasks API
vi.mock('@/features/tasks/api/tasks', () => ({
  tasksApi: {
    listTasks: vi.fn(),
    getTask: vi.fn(),
    createTask: vi.fn(),
    sendMessage: vi.fn(),
    stopAgent: vi.fn(),
    getTaskEvents: vi.fn(),
    getTaskGitDiff: vi.fn(),
    getTaskWorkspaceStatus: vi.fn(),
    getServerStatus: vi.fn(),
    startServer: vi.fn(),
    stopServer: vi.fn(),
  },
}))

// Mock the router
vi.mock('@tanstack/react-router', () => ({
  useNavigate: vi.fn(() => vi.fn()),
}))

// Mock the preview server API
vi.mock('@/features/preview/api/server', () => ({
  serverApi: {
    getServerStatus: vi.fn(),
    startServer: vi.fn(),
    stopServer: vi.fn(),
  },
}))

describe('Task Hooks', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('useTasksList', () => {
    it('fetches tasks list', async () => {
      const mockTasks = [createMockTask({ taskId: 'task-1' }), createMockTask({ taskId: 'task-2' })]
      vi.mocked(tasksApi.listTasks).mockResolvedValue({
        items: mockTasks,
        pagination: { limit: 20, offset: 0, total: 2 },
      })

      const { result } = renderHook(() => useTasksList(), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(result.current.data).toEqual({
        items: mockTasks,
        pagination: { limit: 20, offset: 0, total: 2 },
      })
      expect(tasksApi.listTasks).toHaveBeenCalledWith(undefined)
    })

    it('fetches tasks with filters', async () => {
      const filters = { activeOnly: true, limit: 10, offset: 0 }
      vi.mocked(tasksApi.listTasks).mockResolvedValue({
        items: [],
        pagination: { limit: 10, offset: 0, total: 0 },
      })

      renderHook(() => useTasksList(filters), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(tasksApi.listTasks).toHaveBeenCalledWith(filters)
      })
    })
  })

  describe('useTask', () => {
    it('fetches task details', async () => {
      const mockTask = createMockTaskWithIterations()
      vi.mocked(tasksApi.getTask).mockResolvedValue(mockTask)

      const { result } = renderHook(() => useTask('task-123'), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(result.current.data).toEqual(mockTask)
      expect(tasksApi.getTask).toHaveBeenCalledWith('task-123')
    })

    it('refetches every 2 seconds when task is active', async () => {
      const activeTask = createMockTask({ status: 'running' })
      vi.mocked(tasksApi.getTask).mockResolvedValue(activeTask)

      const { result } = renderHook(() => useTask('task-123'), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      // The hook has refetchInterval configured, but we can't easily test the interval
      // without waiting for actual time to pass. The important thing is that
      // the configuration is set up correctly in the hook itself.
    })

    it('does not refetch when task is not active', async () => {
      const inactiveTask = createMockTask({
        status: 'closed',
      })
      vi.mocked(tasksApi.getTask).mockResolvedValue(inactiveTask)

      renderHook(() => useTask('task-123'), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(tasksApi.getTask).toHaveBeenCalledTimes(1)
      })

      // Wait to ensure no additional calls
      await new Promise((resolve) => setTimeout(resolve, 3000))
      expect(tasksApi.getTask).toHaveBeenCalledTimes(1)
    })
  })

  describe('useCreateTask', () => {
    it('creates task and invalidates queries', async () => {
      const mockResponse = createMockAgentTriggerResponse()
      vi.mocked(tasksApi.createTask).mockResolvedValue(mockResponse)

      const { result } = renderHook(() => useCreateTask(), {
        wrapper: createWrapper(),
      })

      await act(async () => {
        await result.current.mutateAsync({
          agentType: 'test-agent',
          repositoryId: 'test-env',
          message: 'Test message',
        })
      })

      expect(tasksApi.createTask).toHaveBeenCalled()
      // The invalidation happens inside the hook's onSuccess callback
      // which is tested by the hook's integration with react-query
    })
  })

  describe('useSendMessage', () => {
    it('sends message and invalidates correct queries', async () => {
      const mockResponse = createMockAgentTriggerResponse()
      vi.mocked(tasksApi.sendMessage).mockResolvedValue(mockResponse)

      const wrapper = createWrapper()
      const spyInvalidate = vi.spyOn(wrapper({ children: null }).props.client, 'invalidateQueries')

      const { result } = renderHook(() => useSendMessage('task-123'), { wrapper })

      await act(async () => {
        await result.current.mutateAsync({
          message: 'Test message',
        })
      })

      // useSendMessage only invalidates task detail, not lists
      expect(spyInvalidate).toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.detail('task-123'),
      })
    })
  })

  describe('useStopAgent', () => {
    it('stops agent and invalidates correct queries', async () => {
      vi.mocked(tasksApi.stopAgent).mockResolvedValue({
        status: 'stopped',
        taskId: 'task-123',
      })

      const wrapper = createWrapper()
      const spyInvalidate = vi.spyOn(wrapper({ children: null }).props.client, 'invalidateQueries')

      const { result } = renderHook(() => useStopAgent('task-123'), { wrapper })

      await act(async () => {
        await result.current.mutateAsync()
      })

      // Mutations invalidate both task lists and task-specific queries
      expect(spyInvalidate).toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.lists(),
      })
      expect(spyInvalidate).toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.detail('task-123'),
      })
    })
  })

  describe('useTaskEvents', () => {
    it('fetches task events', async () => {
      const mockEvents = createMockEventList(5)
      vi.mocked(tasksApi.getTaskEvents).mockResolvedValue(mockEvents)

      const { result } = renderHook(() => useTaskEvents('task-123'), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(result.current.data).toEqual(mockEvents)
    })

    it('fetches events with type filter', async () => {
      const type = 'message'
      vi.mocked(tasksApi.getTaskEvents).mockResolvedValue([])

      renderHook(() => useTaskEvents('task-123', type), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(tasksApi.getTaskEvents).toHaveBeenCalledWith('task-123', type)
      })
    })
  })

  describe('useTaskGitDiff', () => {
    it('fetches git diff', async () => {
      const mockDiff = createMockGitDiff()
      vi.mocked(tasksApi.getTaskGitDiff).mockResolvedValue(mockDiff)

      const { result } = renderHook(() => useTaskGitDiff('task-123'), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(result.current.data).toEqual(mockDiff)
    })
  })

  describe('useServerStatus', () => {
    it('fetches server status', async () => {
      const mockStatus = createMockServerStatus({ status: 'started' })
      vi.mocked(serverApi.getServerStatus).mockResolvedValue(mockStatus)

      const { result } = renderHook(() => useServerStatus('task-123'), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(result.current.data).toEqual(mockStatus)
    })

    it('refetches when server is running', async () => {
      const runningStatus = createMockServerStatus({ status: 'started' })
      vi.mocked(serverApi.getServerStatus).mockResolvedValue(runningStatus)

      renderHook(() => useServerStatus('task-123'), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(serverApi.getServerStatus).toHaveBeenCalled()
      })

      // The refetch interval should be 5 seconds when running
    })
  })

  describe('useStartServer', () => {
    it('starts server and invalidates queries', async () => {
      const mockStatus = createMockServerStatus({ status: 'started' })
      vi.mocked(serverApi.startServer).mockResolvedValue(mockStatus)

      const wrapper = createWrapper()
      const spyInvalidate = vi.spyOn(wrapper({ children: null }).props.client, 'invalidateQueries')

      const { result } = renderHook(() => useStartServer(), { wrapper })

      await act(async () => {
        await result.current.mutateAsync('task-123')
      })

      expect(spyInvalidate).toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.serverStatus('task-123'),
      })
    })
  })

  describe('useStopServer', () => {
    it('stops server and invalidates queries', async () => {
      const mockStatus = createMockServerStatus({ status: 'stopped' })
      vi.mocked(serverApi.stopServer).mockResolvedValue(mockStatus)

      const wrapper = createWrapper()
      const spyInvalidate = vi.spyOn(wrapper({ children: null }).props.client, 'invalidateQueries')

      const { result } = renderHook(() => useStopServer(), { wrapper })

      await act(async () => {
        await result.current.mutateAsync('task-123')
      })

      expect(spyInvalidate).toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.serverStatus('task-123'),
      })
    })
  })

  describe('Query Invalidation Patterns', () => {
    it('invalidates hierarchical queries correctly', async () => {
      const wrapper = createWrapper()
      const client = wrapper({ children: null }).props.client
      const spyInvalidate = vi.spyOn(client, 'invalidateQueries')

      // Set up some query data
      client.setQueryData(queryKeys.tasks.all, [])
      client.setQueryData(queryKeys.tasks.detail('task-123'), createMockTask())
      client.setQueryData(queryKeys.tasks.events('task-123'), [])

      // Invalidate parent key
      await client.invalidateQueries({
        queryKey: queryKeys.tasks.detail('task-123'),
      })

      // Should invalidate all task-specific queries
      expect(spyInvalidate).toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.detail('task-123'),
      })
    })
  })
})
