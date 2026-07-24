import type { RepositoryResponse } from '@moatless/api-client'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { RepositoryConfigEditSection } from '../repository-config-edit-section'

const mocks = vi.hoisted(() => ({
  syncMutate: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}))

let syncPending = false

const repository: RepositoryResponse = {
  id: 'repo-1',
  kind: 'remote',
  name: 'Moatless',
  provider: 'github',
  remoteUrl: 'https://github.com/aorwall/moatless',
  scope: 'user',
}

vi.mock('sonner', () => ({
  toast: {
    error: mocks.toastError,
    success: mocks.toastSuccess,
  },
}))

vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useConvertToTemplate: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteRepository: () => ({ mutate: vi.fn(), isPending: false }),
  useRepository: () => ({ data: repository, isLoading: false, error: null }),
  useSyncRepositoryConfig: () => ({
    mutate: mocks.syncMutate,
    isPending: syncPending,
  }),
  useUpdateRepository: () => ({ mutate: vi.fn(), isPending: false }),
}))

describe('RepositoryConfigEditSection sync action', () => {
  beforeEach(() => {
    syncPending = false
    mocks.syncMutate.mockReset()
    mocks.toastError.mockReset()
    mocks.toastSuccess.mockReset()
  })

  it('triggers a Git configuration sync and reports success', async () => {
    const user = userEvent.setup()
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    await user.click(screen.getByRole('button', { name: 'Sync from Git' }))

    expect(mocks.syncMutate).toHaveBeenCalledWith(
      'repo-1',
      expect.objectContaining({
        onError: expect.any(Function),
        onSuccess: expect.any(Function),
      }),
    )

    const callbacks = mocks.syncMutate.mock.calls[0]?.[1]
    callbacks.onSuccess()
    expect(mocks.toastSuccess).toHaveBeenCalledWith('Repository configuration synced from Git')
  })

  it('disables the action while a sync is pending', () => {
    syncPending = true

    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    expect(screen.getByRole('button', { name: 'Syncing...' })).toBeDisabled()
  })

  it('reports sync failures', async () => {
    const user = userEvent.setup()
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    await user.click(screen.getByRole('button', { name: 'Sync from Git' }))

    const callbacks = mocks.syncMutate.mock.calls[0]?.[1]
    callbacks.onError(new Error('Git provider unavailable'))
    expect(mocks.toastError).toHaveBeenCalledWith('Failed to sync repository configuration', {
      description: 'Git provider unavailable',
    })
  })
})
