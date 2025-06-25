import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  createMockCreateEnvironmentRequest,
  createMockEnvironment,
  createMockEnvironmentList,
  createMockUpdateEnvironmentRequest,
} from '@/test-utils/factories'
import { createWrapper } from '@/test-utils/render'
import { environmentsApi } from '../../api/environments'
import {
  useBranches,
  useCreateEnvironment,
  useDeleteEnvironment,
  useEnvironment,
  useEnvironmentsList,
  useRunSetupScript,
  useSearchFiles,
  useUpdateEnvironment,
} from '../use-environments'

// Mock the environments API
vi.mock('../../api/environments', () => ({
  environmentsApi: {
    listEnvironments: vi.fn(),
    getEnvironment: vi.fn(),
    createEnvironment: vi.fn(),
    updateEnvironment: vi.fn(),
    deleteEnvironment: vi.fn(),
    runSetupScript: vi.fn(),
    searchFiles: vi.fn(),
    getBranches: vi.fn(),
  },
}))

describe('Environment Hooks', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('useEnvironmentsList', () => {
    it('should fetch environments list', async () => {
      const mockEnvironments = createMockEnvironmentList(3)
      vi.mocked(environmentsApi.listEnvironments).mockResolvedValueOnce(mockEnvironments)

      const { result } = renderHook(() => useEnvironmentsList(), {
        wrapper: createWrapper(),
      })

      expect(result.current.isLoading).toBe(true)

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      expect(result.current.data).toEqual(mockEnvironments)
      expect(environmentsApi.listEnvironments).toHaveBeenCalledTimes(1)
    })

    it('should handle errors when fetching environments', async () => {
      const error = new Error('Failed to fetch environments')
      vi.mocked(environmentsApi.listEnvironments).mockRejectedValueOnce(error)

      const { result } = renderHook(() => useEnvironmentsList(), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(result.current.isError).toBe(true)
      })

      expect(result.current.error).toEqual(error)
    })
  })

  describe('useEnvironment', () => {
    it('should fetch single environment', async () => {
      const mockEnvironment = createMockEnvironment()
      vi.mocked(environmentsApi.getEnvironment).mockResolvedValueOnce(mockEnvironment)

      const { result } = renderHook(() => useEnvironment('env-123'), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      expect(result.current.data).toEqual(mockEnvironment)
      expect(environmentsApi.getEnvironment).toHaveBeenCalledWith('env-123')
    })

    it('should not fetch if envId is empty', () => {
      renderHook(() => useEnvironment(''), {
        wrapper: createWrapper(),
      })

      expect(environmentsApi.getEnvironment).not.toHaveBeenCalled()
    })
  })

  describe('useCreateEnvironment', () => {
    it('should create environment and invalidate queries', async () => {
      const mockRequest = createMockCreateEnvironmentRequest()
      const mockResponse = createMockEnvironment()
      vi.mocked(environmentsApi.createEnvironment).mockResolvedValueOnce(mockResponse)

      const onSuccess = vi.fn()
      const wrapper = createWrapper()
      const { result } = renderHook(
        () =>
          useCreateEnvironment({
            onSuccess,
          }),
        { wrapper },
      )

      await act(async () => {
        result.current.mutate(mockRequest)
      })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(environmentsApi.createEnvironment).toHaveBeenCalledWith(mockRequest)
      expect(onSuccess).toHaveBeenCalledWith(mockResponse, mockRequest, undefined)
    })

    it('should handle creation errors', async () => {
      const error = new Error('Failed to create environment')
      vi.mocked(environmentsApi.createEnvironment).mockRejectedValueOnce(error)

      const onError = vi.fn()
      const { result } = renderHook(
        () =>
          useCreateEnvironment({
            onError,
          }),
        { wrapper: createWrapper() },
      )

      await act(async () => {
        result.current.mutate(createMockCreateEnvironmentRequest())
      })

      await waitFor(() => {
        expect(result.current.isError).toBe(true)
      })

      expect(result.current.error).toEqual(error)
      expect(onError).toHaveBeenCalled()
    })
  })

  describe('useUpdateEnvironment', () => {
    it('should update environment and invalidate queries', async () => {
      const mockRequest = createMockUpdateEnvironmentRequest()
      const mockResponse = createMockEnvironment({ name: mockRequest.name })
      vi.mocked(environmentsApi.updateEnvironment).mockResolvedValueOnce(mockResponse)

      const onSuccess = vi.fn()
      const { result } = renderHook(
        () =>
          useUpdateEnvironment({
            onSuccess,
          }),
        { wrapper: createWrapper() },
      )

      const params = { envId: 'env-123', request: mockRequest }

      await act(async () => {
        result.current.mutate(params)
      })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(environmentsApi.updateEnvironment).toHaveBeenCalledWith('env-123', mockRequest)
      expect(onSuccess).toHaveBeenCalledWith(mockResponse, params, undefined)
    })
  })

  describe('useDeleteEnvironment', () => {
    it('should delete environment and invalidate queries', async () => {
      vi.mocked(environmentsApi.deleteEnvironment).mockResolvedValueOnce(undefined)

      const onSuccess = vi.fn()
      const { result } = renderHook(
        () =>
          useDeleteEnvironment({
            onSuccess,
          }),
        { wrapper: createWrapper() },
      )

      await act(async () => {
        result.current.mutate('env-123')
      })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(environmentsApi.deleteEnvironment).toHaveBeenCalledWith('env-123')
      expect(onSuccess).toHaveBeenCalledWith(undefined, 'env-123', undefined)
    })
  })

  describe('useRunSetupScript', () => {
    it('should run setup script', async () => {
      const mockResponse = { success: true, output: 'Setup completed' }
      vi.mocked(environmentsApi.runSetupScript).mockResolvedValueOnce(mockResponse)

      const { result } = renderHook(() => useRunSetupScript(), {
        wrapper: createWrapper(),
      })

      await act(async () => {
        result.current.mutate('env-123')
      })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(result.current.data).toEqual(mockResponse)
      expect(environmentsApi.runSetupScript).toHaveBeenCalledWith('env-123')
    })
  })

  describe('useSearchFiles', () => {
    it('should search files when query is valid', async () => {
      const mockResponse = {
        files: ['/src/index.ts', '/src/app.ts'],
        totalCount: 2,
        truncated: false,
      }
      vi.mocked(environmentsApi.searchFiles).mockResolvedValueOnce(mockResponse)

      const { result } = renderHook(() => useSearchFiles('env-123', 'test'), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      expect(result.current.data).toEqual(mockResponse)
      expect(environmentsApi.searchFiles).toHaveBeenCalledWith('env-123', {
        query: 'test',
        limit: 10,
      })
    })

    it('should not search if query is too short', () => {
      renderHook(() => useSearchFiles('env-123', 'te'), {
        wrapper: createWrapper(),
      })

      expect(environmentsApi.searchFiles).not.toHaveBeenCalled()
    })

    it('should not search if envId is empty', () => {
      renderHook(() => useSearchFiles('', 'test'), {
        wrapper: createWrapper(),
      })

      expect(environmentsApi.searchFiles).not.toHaveBeenCalled()
    })
  })

  describe('useBranches', () => {
    it('should fetch branches', async () => {
      const mockResponse = {
        branches: ['main', 'develop', 'feature/test'],
        currentBranch: 'main',
      }
      vi.mocked(environmentsApi.getBranches).mockResolvedValueOnce(mockResponse)

      const { result } = renderHook(() => useBranches('env-123'), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      expect(result.current.data).toEqual(mockResponse)
      expect(environmentsApi.getBranches).toHaveBeenCalledWith('env-123')
    })

    it('should not fetch if envId is empty', () => {
      renderHook(() => useBranches(''), {
        wrapper: createWrapper(),
      })

      expect(environmentsApi.getBranches).not.toHaveBeenCalled()
    })
  })
})
