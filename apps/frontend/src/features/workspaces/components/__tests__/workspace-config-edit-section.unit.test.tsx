import type { WorkspaceResponse } from '@moatless/api-client'
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { WorkspaceConfigEditSection } from '../workspace-config-edit-section'

const updateMutate = vi.fn()
const deleteMutate = vi.fn()

function workspace(overrides: Partial<WorkspaceResponse>): WorkspaceResponse {
  return {
    id: 'ws-1',
    name: 'Synced Workspace',
    kind: 'single',
    scope: 'team',
    repos: [],
    ...overrides,
  }
}

let currentWorkspace: WorkspaceResponse

vi.mock('@/features/workspaces/hooks/use-workspaces', () => ({
  useWorkspace: () => ({ data: currentWorkspace, isLoading: false, error: null }),
  useUpdateWorkspace: () => ({ mutate: updateMutate, isPending: false }),
  useDeleteWorkspace: () => ({ mutate: deleteMutate, isPending: false }),
  useAddWorkspaceRepo: () => ({ mutate: vi.fn(), isPending: false }),
  useRemoveWorkspaceRepo: () => ({ mutate: vi.fn(), isPending: false }),
  useSetWorkspacePrimaryRepo: () => ({ mutate: vi.fn(), isPending: false }),
}))

vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoriesList: () => ({ data: [] }),
}))

vi.mock('@/features/plugins/hooks/use-plugins', () => ({
  usePlugins: () => ({ data: { plugins: [] }, isLoading: false }),
}))

describe('WorkspaceConfigEditSection (git-synced)', () => {
  it('shows the read-only alert and hides mutating controls', () => {
    currentWorkspace = workspace({ source: 'git', syncedFromGit: true })

    render(<WorkspaceConfigEditSection workspaceId="ws-1" />)

    // The read-only banner is shown.
    expect(screen.getByText('Synced from Git')).toBeInTheDocument()

    // Save and Delete are unreachable.
    expect(screen.queryByRole('button', { name: /save changes/i })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /delete workspace/i })).not.toBeInTheDocument()

    // The name input renders the value but is disabled.
    expect(screen.getByDisplayValue('Synced Workspace')).toBeDisabled()
  })

  it('shows the action bar and editable inputs for a manual workspace', () => {
    currentWorkspace = workspace({ name: 'Manual Workspace', source: 'manual' })

    render(<WorkspaceConfigEditSection workspaceId="ws-1" />)

    expect(screen.queryByText('Synced from Git')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /save changes/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /delete workspace/i })).toBeInTheDocument()
    expect(screen.getByDisplayValue('Manual Workspace')).not.toBeDisabled()
  })
})
