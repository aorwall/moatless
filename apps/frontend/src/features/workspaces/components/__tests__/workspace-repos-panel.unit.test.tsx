import type { WorkspaceRepoResponse } from '@moatless/api-client'
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { WorkspaceReposPanel } from '../workspace-repos-panel'

const addMutate = vi.fn()
const removeMutate = vi.fn()
const setPrimaryMutate = vi.fn()

vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoriesList: () => ({
    data: [
      { id: 'repo-api', name: 'api', kind: 'remote' },
      { id: 'repo-web', name: 'web', kind: 'remote' },
      { id: 'repo-tmpl', name: 'template-repo', kind: 'template' },
    ],
  }),
}))

vi.mock('@/features/workspaces/hooks/use-workspaces', () => ({
  useAddWorkspaceRepo: () => ({ mutate: addMutate, isPending: false }),
  useRemoveWorkspaceRepo: () => ({ mutate: removeMutate, isPending: false }),
  useSetWorkspacePrimaryRepo: () => ({ mutate: setPrimaryMutate, isPending: false }),
}))

function placement(overrides: Partial<WorkspaceRepoResponse>): WorkspaceRepoResponse {
  return {
    id: 'wsr-1',
    workspaceId: 'ws-1',
    repositoryId: 'repo-api',
    branch: null,
    mountName: null,
    isPrimary: false,
    position: 0,
    createdAt: null,
    ...overrides,
  }
}

describe('WorkspaceReposPanel', () => {
  beforeEach(() => {
    addMutate.mockClear()
    removeMutate.mockClear()
    setPrimaryMutate.mockClear()
  })

  it('shows the blank-workspace empty state when there are no placements', () => {
    render(<WorkspaceReposPanel workspaceId="ws-1" placements={[]} />)
    expect(screen.getByText(/blank workspace/i)).toBeInTheDocument()
  })

  it('maps repository ids to names and marks the primary placement', () => {
    render(
      <WorkspaceReposPanel
        workspaceId="ws-1"
        placements={[
          placement({ id: 'wsr-1', repositoryId: 'repo-api', isPrimary: true }),
          placement({
            id: 'wsr-2',
            repositoryId: 'repo-web',
            branch: 'dev',
            mountName: 'frontend',
          }),
        ]}
      />,
    )

    expect(screen.getByText('api')).toBeInTheDocument()
    expect(screen.getByText('web')).toBeInTheDocument()
    expect(screen.getByText('Primary')).toBeInTheDocument()
    // Per-placement branch + mount name are surfaced.
    expect(screen.getByText('dev')).toBeInTheDocument()
    expect(screen.getByText('./frontend')).toBeInTheDocument()
  })

  it('sets a non-primary placement as primary', () => {
    render(
      <WorkspaceReposPanel
        workspaceId="ws-1"
        placements={[
          placement({ id: 'wsr-1', repositoryId: 'repo-api', isPrimary: true }),
          placement({ id: 'wsr-2', repositoryId: 'repo-web' }),
        ]}
      />,
    )

    fireEvent.click(screen.getByTitle('Set as primary'))
    expect(setPrimaryMutate).toHaveBeenCalledWith(
      { workspaceId: 'ws-1', placementId: 'wsr-2' },
      expect.anything(),
    )
  })

  it('removes a placement', () => {
    render(
      <WorkspaceReposPanel
        workspaceId="ws-1"
        placements={[placement({ id: 'wsr-2', repositoryId: 'repo-web' })]}
      />,
    )

    fireEvent.click(screen.getByLabelText('Remove web'))
    expect(removeMutate).toHaveBeenCalledWith(
      { workspaceId: 'ws-1', placementId: 'wsr-2' },
      expect.anything(),
    )
  })
})
