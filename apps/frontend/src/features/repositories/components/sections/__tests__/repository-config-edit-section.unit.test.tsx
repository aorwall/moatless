import type { RepositoryResponse, SyncConfigResponse } from '@moatless/api-client'
import userEvent from '@testing-library/user-event'
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@/test-utils/render'
import { RepositoryConfigEditSection } from '../repository-config-edit-section'

const mocks = vi.hoisted(() => ({
  syncMutate: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  toastWarning: vi.fn(),
  updateMutate: vi.fn(),
}))

let currentRepository: RepositoryResponse
let syncPending = false

beforeAll(() => {
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: vi.fn(),
  })
})

vi.mock('sonner', () => ({
  toast: {
    error: mocks.toastError,
    success: mocks.toastSuccess,
    warning: mocks.toastWarning,
  },
}))

vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepository: () => ({ data: currentRepository, isLoading: false, error: null }),
  useUpdateRepository: () => ({ mutate: mocks.updateMutate, isPending: false }),
  useDeleteRepository: () => ({ mutate: vi.fn(), isPending: false }),
  useConvertToTemplate: () => ({ mutate: vi.fn(), isPending: false }),
  useSyncRepositoryConfig: () => ({
    mutate: mocks.syncMutate,
    isPending: syncPending,
  }),
}))

vi.mock('@/features/admin/hooks/use-admin-users-teams', () => ({
  useAdminUsersList: () => ({
    data: {
      users: [{ id: 'owner-1', login: 'alice', name: 'Alice Example' }],
    },
  }),
}))

function repository(overrides: Partial<RepositoryResponse> = {}): RepositoryResponse {
  return {
    id: 'repo-1',
    name: 'Repository',
    kind: 'remote',
    remote: { url: 'https://github.com/acme/repo.git', usesSsh: false, hostRepo: null },
    provider: 'github',
    scope: 'user',
    ownerUserId: 'owner-1',
    ...overrides,
  }
}

/** A sync-config result, optionally reporting reclaimed workspace overrides. */
function syncResult(reclaimed = 0): SyncConfigResponse {
  return {
    repository: currentRepository,
    loops: {
      created: 0,
      overridesSkipped: 0,
      removed: 0,
      skipped: [],
      updated: 0,
      warnings: [],
    },
    workspaces: { created: 0, reclaimed, removed: 0, skipped: [], updated: 0, warnings: [] },
  }
}

describe('RepositoryConfigEditSection', () => {
  beforeEach(() => {
    currentRepository = repository()
    syncPending = false
    for (const mock of Object.values(mocks)) {
      mock.mockReset()
    }
  })

  it('submits only changed repository fields', async () => {
    const user = userEvent.setup()
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    const name = screen.getByDisplayValue('Repository')
    await user.clear(name)
    await user.type(name, 'Renamed')
    await user.click(screen.getByRole('button', { name: 'Save Changes' }))

    expect(mocks.updateMutate).toHaveBeenCalledWith(
      {
        repoId: 'repo-1',
        request: { name: 'Renamed' },
      },
      expect.any(Object),
    )
  })

  it('disables User scope for a repository without an owner', async () => {
    const user = userEvent.setup()
    currentRepository = repository({
      scope: 'global',
      ownerUserId: null,
    })
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    screen.getByLabelText('Repository access scope').focus()
    await user.keyboard('{ArrowDown}')

    expect(screen.getByRole('option', { name: 'User' })).toHaveAttribute('aria-disabled', 'true')
  })

  it('triggers a Git configuration sync and reports success', async () => {
    const user = userEvent.setup()
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    await user.click(screen.getByRole('button', { name: 'Sync from Git' }))
    await user.click(screen.getByRole('button', { name: 'Synchronize' }))

    expect(mocks.syncMutate).toHaveBeenCalledWith(
      'repo-1',
      expect.objectContaining({
        onError: expect.any(Function),
        onSuccess: expect.any(Function),
      }),
    )

    const callbacks = mocks.syncMutate.mock.calls[0]?.[1]
    callbacks.onSuccess(syncResult())
    expect(mocks.toastSuccess).toHaveBeenCalledWith('Repository configuration synced from Git', {
      description: '',
    })
  })

  it('reports workspaces whose override the sync reclaimed', async () => {
    const user = userEvent.setup()
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    await user.click(screen.getByRole('button', { name: 'Sync from Git' }))
    await user.click(screen.getByRole('button', { name: 'Synchronize' }))

    const callbacks = mocks.syncMutate.mock.calls[0]?.[1]
    callbacks.onSuccess(syncResult(2))
    expect(mocks.toastSuccess).toHaveBeenCalledWith('Repository configuration synced from Git', {
      description: '2 overridden workspaces reset to Git.',
    })
  })

  it('reports Loop changes, skips, and warnings from Git sync', async () => {
    const user = userEvent.setup()
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    await user.click(screen.getByRole('button', { name: 'Sync from Git' }))
    await user.click(screen.getByRole('button', { name: 'Synchronize' }))

    const result = syncResult()
    result.loops = {
      created: 1,
      overridesSkipped: 1,
      removed: 1,
      skipped: [{ loopName: 'local', reason: 'local override' }],
      updated: 2,
      warnings: [{ loopName: 'daily', field: 'active', message: 'ignored' }],
    }
    const callbacks = mocks.syncMutate.mock.calls[0]?.[1]
    callbacks.onSuccess(result)
    expect(mocks.toastSuccess).toHaveBeenCalledWith('Repository configuration synced from Git', {
      description:
        '1 Loop created. 2 Loops updated. 1 Loop removed. 1 Loop override preserved. 1 Loop skipped. 1 Loop warning reported.',
    })
  })

  it('reports the workspace keys the sync ignored, rather than plain success', async () => {
    const user = userEvent.setup()
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    await user.click(screen.getByRole('button', { name: 'Sync from Git' }))
    await user.click(screen.getByRole('button', { name: 'Synchronize' }))

    const result = syncResult()
    result.workspaces.warnings = [
      { path: 'workspaces[0].previewConfig', message: 'unknown key, ignored' },
    ]
    const callbacks = mocks.syncMutate.mock.calls[0]?.[1]
    callbacks.onSuccess(result)

    expect(mocks.toastWarning).toHaveBeenCalledWith('Synced from Git, ignoring 1 declared key', {
      description: 'workspaces[0].previewConfig: unknown key, ignored',
    })
    expect(mocks.toastSuccess).not.toHaveBeenCalled()
  })

  it('disables the sync action while pending', () => {
    syncPending = true

    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    expect(screen.getByRole('button', { name: 'Syncing...' })).toBeDisabled()
  })

  it('reports sync failures', async () => {
    const user = userEvent.setup()
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    await user.click(screen.getByRole('button', { name: 'Sync from Git' }))
    await user.click(screen.getByRole('button', { name: 'Synchronize' }))

    const callbacks = mocks.syncMutate.mock.calls[0]?.[1]
    callbacks.onError(new Error('Git provider unavailable'))
    expect(mocks.toastError).toHaveBeenCalledWith('Failed to sync repository configuration', {
      description: 'Git provider unavailable',
    })
  })
})
