import type { SyncConfigResponse } from '@moatless/api-client'
import { QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { repositoriesApi } from '@/features/repositories/api/repositories'
import { queryKeys } from '@/lib/api/query-keys'
import { createTestQueryClient } from '@/test-utils/render'
import { useSyncRepositoryConfig } from '../use-repositories'

vi.mock('@/features/repositories/api/repositories', () => ({
  repositoriesApi: {
    syncConfig: vi.fn(),
  },
}))

const syncResponse: SyncConfigResponse = {
  repository: {
    id: 'repo-1',
    remote: null,
    kind: 'remote',
    name: 'Moatless',
    scope: 'user',
  },
  loops: {
    created: 0,
    overridesSkipped: 0,
    removed: 0,
    skipped: [],
    updated: 0,
    warnings: [],
  },
  workspaces: {
    created: 0,
    reclaimed: 0,
    removed: 0,
    skipped: [],
    updated: 0,
    warnings: [],
  },
}

describe('useSyncRepositoryConfig', () => {
  beforeEach(() => {
    vi.mocked(repositoriesApi.syncConfig).mockReset()
  })

  it('syncs the repository and invalidates affected configuration queries', async () => {
    const queryClient = createTestQueryClient()
    const invalidateQueries = vi.spyOn(queryClient, 'invalidateQueries')
    vi.mocked(repositoriesApi.syncConfig).mockResolvedValue(syncResponse)

    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    )
    const { result } = renderHook(() => useSyncRepositoryConfig(), { wrapper })

    await act(async () => {
      await result.current.mutateAsync('repo-1')
    })

    expect(repositoriesApi.syncConfig).toHaveBeenCalledWith('repo-1')
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: queryKeys.repositories.all(),
    })
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: queryKeys.workspaces.all(),
    })
    expect(invalidateQueries).toHaveBeenCalledWith({
      queryKey: queryKeys.loops.all(),
    })
  })
})
