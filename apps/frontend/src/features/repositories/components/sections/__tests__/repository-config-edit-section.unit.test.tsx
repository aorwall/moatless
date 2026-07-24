import type { RepositoryResponse } from '@moatless/api-client'
import userEvent from '@testing-library/user-event'
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@/test-utils/render'
import { RepositoryConfigEditSection } from '../repository-config-edit-section'

const updateMutate = vi.fn()
const verifyMutate = vi.fn()

let currentRepository: RepositoryResponse

beforeAll(() => {
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: vi.fn(),
  })
})

vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepository: () => ({ data: currentRepository, isLoading: false, error: null }),
  useUpdateRepository: () => ({ mutate: updateMutate, isPending: false }),
  useVerifyRepositoryAccess: () => ({ mutate: verifyMutate, isPending: false }),
  useDeleteRepository: () => ({ mutate: vi.fn(), isPending: false }),
  useConvertToTemplate: () => ({ mutate: vi.fn(), isPending: false }),
}))

vi.mock('@/features/admin/hooks/use-admin-users-teams', () => ({
  useAdminTeamsList: () => ({
    data: [
      { id: 'team-1', name: 'Original', slug: 'original' },
      { id: 'team-2', name: 'Platform', slug: 'platform' },
    ],
    isLoading: false,
  }),
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
    remoteUrl: 'https://github.com/acme/repo.git',
    provider: 'github',
    scope: 'user',
    teamId: 'team-1',
    ownerUserId: 'owner-1',
    accessStatus: 'unchecked',
    ...overrides,
  }
}

describe('RepositoryConfigEditSection access editing', () => {
  beforeEach(() => {
    currentRepository = repository()
    updateMutate.mockReset()
    verifyMutate.mockReset()
  })

  it('submits only changed repository fields', async () => {
    const user = userEvent.setup()
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    const name = screen.getByDisplayValue('Repository')
    await user.clear(name)
    await user.type(name, 'Renamed')
    await user.click(screen.getByRole('button', { name: 'Save Changes' }))

    expect(updateMutate).toHaveBeenCalledWith(
      {
        repoId: 'repo-1',
        request: { name: 'Renamed' },
      },
      expect.any(Object),
    )
  })

  it('requires a target team and sends the selected scope cleanly', async () => {
    const user = userEvent.setup()
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    screen.getByLabelText('Repository access scope').focus()
    await user.keyboard('{ArrowDown}')
    await user.click(screen.getByRole('option', { name: 'Team' }))

    expect(screen.getByRole('button', { name: 'Save Changes' })).toBeDisabled()
    expect(screen.getByText('Select a team to save.')).toBeInTheDocument()

    screen.getByLabelText('Repository access team').focus()
    await user.keyboard('{ArrowDown}')
    await user.click(screen.getByRole('option', { name: 'Platform (platform)' }))
    await user.click(screen.getByRole('button', { name: 'Save Changes' }))

    expect(updateMutate).toHaveBeenCalledWith(
      {
        repoId: 'repo-1',
        request: { scope: 'team', teamId: 'team-2' },
      },
      expect.any(Object),
    )
  })

  it('disables User scope for a repository without an owner', async () => {
    const user = userEvent.setup()
    currentRepository = repository({
      scope: 'global',
      teamId: 'system',
      ownerUserId: null,
    })
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    screen.getByLabelText('Repository access scope').focus()
    await user.keyboard('{ArrowDown}')

    expect(screen.getByRole('option', { name: 'User' })).toHaveAttribute('aria-disabled', 'true')
  })

  it('keeps verification disabled until edits are saved or discarded', async () => {
    const user = userEvent.setup()
    render(<RepositoryConfigEditSection repositoryId="repo-1" />)

    expect(screen.getByRole('button', { name: 'Verify access' })).toBeEnabled()
    await user.type(screen.getByDisplayValue('Repository'), ' changed')
    expect(screen.getByRole('button', { name: 'Verify access' })).toBeDisabled()
  })
})
