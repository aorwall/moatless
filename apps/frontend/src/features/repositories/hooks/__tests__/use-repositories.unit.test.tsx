import type { RepositoryResponse } from '@moatless/types/types/repository'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { repositoriesApi } from '@/features/repositories/api/repositories'
import {
  useBranches,
  useCreateRepository,
  useDeleteRepository,
  useRepositoriesList,
  useRepository,
  useSearchFiles,
  useUpdateRepository,
} from '../use-repositories'

// Mock the API
vi.mock('@/features/repositories/api/repositories', () => ({
  repositoriesApi: {
    listRepositories: vi.fn(),
    getRepository: vi.fn(),
    createRepository: vi.fn(),
    updateRepository: vi.fn(),
    deleteRepository: vi.fn(),
    runSetupScript: vi.fn(),
    searchFiles: vi.fn(),
    getBranches: vi.fn(),
  },
}))

describe('useRepositories hooks', () => {
  let queryClient: QueryClient

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )

  const mockRepository: RepositoryResponse = {
    id: 'repo-1',
    name: 'Test Repository',
    description: 'Test description',
    path: '/test/repo',
    useGitWorktrees: true,
    originalBranch: 'main',
    codeExecution: {
      type: 'local' as const,
    },
    previewConfig: {
      framework: 'vite',
      serverStartCommand: 'npm run dev',
    },
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('useRepositoriesList', () => {
    it('should fetch repositories list', async () => {
      const mockRepositories = [mockRepository]
      vi.mocked(repositoriesApi.listRepositories).mockResolvedValue(mockRepositories)

      const { result } = renderHook(() => useRepositoriesList(), { wrapper })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(result.current.data).toEqual(mockRepositories)
      expect(repositoriesApi.listRepositories).toHaveBeenCalledTimes(1)
    })

    it('should handle error when fetching repositories fails', async () => {
      const error = new Error('Failed to fetch')
      vi.mocked(repositoriesApi.listRepositories).mockRejectedValue(error)

      const { result } = renderHook(() => useRepositoriesList(), { wrapper })

      await waitFor(() => {
        expect(result.current.isError).toBe(true)
      })

      expect(result.current.error).toBe(error)
    })
  })

  describe('useRepository', () => {
    it('should fetch single repository', async () => {
      vi.mocked(repositoriesApi.getRepository).mockResolvedValue(mockRepository)

      const { result } = renderHook(() => useRepository('repo-1'), { wrapper })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(result.current.data).toEqual(mockRepository)
      expect(repositoriesApi.getRepository).toHaveBeenCalledWith('repo-1')
    })

    it('should not fetch when repoId is empty', () => {
      const { result } = renderHook(() => useRepository(''), { wrapper })

      expect(result.current.isPending).toBe(true)
      expect(repositoriesApi.getRepository).not.toHaveBeenCalled()
    })
  })

  describe('useCreateRepository', () => {
    it('should create repository and invalidate queries', async () => {
      vi.mocked(repositoriesApi.createRepository).mockResolvedValue(mockRepository)

      const onSuccess = vi.fn()
      const { result } = renderHook(() => useCreateRepository({ onSuccess }), { wrapper })

      const createRequest = {
        name: 'New Repo',
        description: 'New description',
        path: '/new/repo',
        useGitWorktrees: true,
        originalBranch: 'main',
        codeExecution: {
          type: 'local' as const,
        },
      }

      result.current.mutate(createRequest)

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(repositoriesApi.createRepository).toHaveBeenCalledWith(createRequest)
      expect(onSuccess).toHaveBeenCalledWith(mockRepository, createRequest, undefined)
    })

    it('should handle creation error', async () => {
      const error = new Error('Creation failed')
      vi.mocked(repositoriesApi.createRepository).mockRejectedValue(error)

      const onError = vi.fn()
      const { result } = renderHook(() => useCreateRepository({ onError }), { wrapper })

      result.current.mutate({
        name: 'New Repo',
        path: '/new/repo',
        codeExecution: { type: 'local' as const },
      })

      await waitFor(() => {
        expect(result.current.isError).toBe(true)
      })

      expect(result.current.error).toBe(error)
      expect(onError).toHaveBeenCalled()
    })
  })

  describe('useUpdateRepository', () => {
    it('should update repository and invalidate queries', async () => {
      const updatedRepo = { ...mockRepository, name: 'Updated Name' }
      vi.mocked(repositoriesApi.updateRepository).mockResolvedValue(updatedRepo)

      const onSuccess = vi.fn()
      const { result } = renderHook(() => useUpdateRepository({ onSuccess }), { wrapper })

      const updateData = {
        repoId: 'repo-1',
        request: { name: 'Updated Name' },
      }

      result.current.mutate(updateData)

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(repositoriesApi.updateRepository).toHaveBeenCalledWith('repo-1', {
        name: 'Updated Name',
      })
      expect(onSuccess).toHaveBeenCalledWith(updatedRepo, updateData, undefined)
    })
  })

  describe('useDeleteRepository', () => {
    it('should delete repository and invalidate queries', async () => {
      vi.mocked(repositoriesApi.deleteRepository).mockResolvedValue(undefined)

      const onSuccess = vi.fn()
      const { result } = renderHook(() => useDeleteRepository({ onSuccess }), { wrapper })

      result.current.mutate('repo-1')

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(repositoriesApi.deleteRepository).toHaveBeenCalledWith('repo-1')
      expect(onSuccess).toHaveBeenCalledWith(undefined, 'repo-1', undefined)
    })
  })

  describe('useSearchFiles', () => {
    it('should search files when query is valid', async () => {
      const mockResponse = {
        results: [
          { path: 'src/index.ts', absolutePath: '/test/repo/src/index.ts', score: 1 },
          { path: 'src/app.ts', absolutePath: '/test/repo/src/app.ts', score: 1 },
        ],
        totalFound: 2,
        query: 'index',
      }
      vi.mocked(repositoriesApi.searchFiles).mockResolvedValue(mockResponse)

      const { result } = renderHook(() => useSearchFiles('repo-1', 'index'), { wrapper })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(result.current.data).toEqual(mockResponse)
      expect(repositoriesApi.searchFiles).toHaveBeenCalledWith('repo-1', {
        repositoryId: 'repo-1',
        query: 'index',
        limit: 10,
      })
    })

    it('should not search when query is too short', () => {
      const { result } = renderHook(() => useSearchFiles('repo-1', 'in'), { wrapper })

      expect(result.current.isPending).toBe(true)
      expect(repositoriesApi.searchFiles).not.toHaveBeenCalled()
    })

    it('should not search when repoId is empty', () => {
      const { result } = renderHook(() => useSearchFiles('', 'index'), { wrapper })

      expect(result.current.isPending).toBe(true)
      expect(repositoriesApi.searchFiles).not.toHaveBeenCalled()
    })
  })

  describe('useBranches', () => {
    it('should fetch branches list', async () => {
      const mockBranches = {
        branches: ['main', 'develop', 'feature/test'],
        currentBranch: 'main',
      }
      vi.mocked(repositoriesApi.getBranches).mockResolvedValue(mockBranches)

      const { result } = renderHook(() => useBranches('repo-1'), { wrapper })

      await waitFor(() => {
        expect(result.current.isSuccess).toBe(true)
      })

      expect(result.current.data).toEqual(mockBranches)
      expect(repositoriesApi.getBranches).toHaveBeenCalledWith('repo-1')
    })

    it('should not fetch branches when repoId is empty', () => {
      const { result } = renderHook(() => useBranches(''), { wrapper })

      expect(result.current.isPending).toBe(true)
      expect(repositoriesApi.getBranches).not.toHaveBeenCalled()
    })
  })
})
