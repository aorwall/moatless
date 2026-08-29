import type { WorkspaceResponse } from '@moatless/api-client'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { WorkspaceConfigEditSection } from '../workspace-config-edit-section'

const updateMutate = vi.fn()
const deleteMutate = vi.fn()
const overrideMutate = vi.fn()
const resetGitMutate = vi.fn()

function workspace(overrides: Partial<WorkspaceResponse>): WorkspaceResponse {
  return {
    id: 'ws-1',
    name: 'Synced Workspace',
    kind: 'single',
    scope: 'user',
    repos: [],
    ...overrides,
  }
}

/** A workspace declared in git, with the provenance an override preserves. */
function gitSourced(overrides: Partial<WorkspaceResponse> = {}): WorkspaceResponse {
  return workspace({
    source: 'git',
    syncedFromGit: true,
    sourceRepositoryId: 'repo-1',
    sourceKey: 'api',
    sourceConfigPath: '.moatless/workspaces.json',
    ...overrides,
  })
}

let currentWorkspace: WorkspaceResponse

vi.mock('@/features/workspaces/hooks/use-workspaces', () => ({
  useWorkspace: () => ({ data: currentWorkspace, isLoading: false, error: null }),
  useUpdateWorkspace: () => ({ mutate: updateMutate, isPending: false }),
  useDeleteWorkspace: () => ({ mutate: deleteMutate, isPending: false }),
  useOverrideWorkspace: () => ({ mutate: overrideMutate, isPending: false }),
  useResetGitWorkspace: () => ({ mutate: resetGitMutate, isPending: false }),
  useAddWorkspaceRepo: () => ({ mutate: vi.fn(), isPending: false }),
  useRemoveWorkspaceRepo: () => ({ mutate: vi.fn(), isPending: false }),
  useSetWorkspacePrimaryRepo: () => ({ mutate: vi.fn(), isPending: false }),
}))

vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoriesList: () => ({ data: [] }),
  useRepository: () => ({ data: { id: 'repo-1', name: 'acme-api' } }),
}))

// The skills panel reads the plugin registry. These tests are about the
// provenance and save controls around it, so it stands in as an empty registry
// rather than pulling a QueryClient into every case.
vi.mock('@/features/plugins/hooks/use-plugins', () => ({
  usePlugins: () => ({ data: [], isLoading: false, isError: false }),
  usePluginSkills: () => ({ data: [], isLoading: false, isError: false }),
  usePluginActivations: () => ({ data: [] }),
  useEffectivePlugins: () => ({ data: [] }),
  useSetActivation: () => ({ mutate: vi.fn(), isPending: false }),
  useClearActivation: () => ({ mutate: vi.fn(), isPending: false }),
}))

beforeEach(() => {
  updateMutate.mockReset()
  deleteMutate.mockReset()
  overrideMutate.mockReset()
  resetGitMutate.mockReset()
})

describe('WorkspaceConfigEditSection (git-synced)', () => {
  it('shows the read-only alert and hides editing controls', () => {
    currentWorkspace = gitSourced()

    render(<WorkspaceConfigEditSection workspaceId="ws-1" />)

    // The read-only banner names where the config comes from.
    expect(screen.getByText('Synced from Git')).toBeInTheDocument()
    expect(screen.getByText(/acme-api\/\.moatless\/workspaces\.json/)).toBeInTheDocument()

    // Editing is unreachable, but deleting creates a restorable tombstone.
    expect(screen.queryByRole('button', { name: /save changes/i })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /delete workspace/i })).toBeInTheDocument()

    // The name input renders the value but is disabled.
    expect(screen.getByDisplayValue('Synced Workspace')).toBeDisabled()
  })

  it('offers taking the workspace under manual control without changing fields', async () => {
    currentWorkspace = gitSourced()
    const user = userEvent.setup()

    render(<WorkspaceConfigEditSection workspaceId="ws-1" />)
    await user.click(screen.getByRole('button', { name: /override to edit/i }))

    // An empty patch flips `source` only — the normal save path handles edits.
    expect(overrideMutate).toHaveBeenCalledWith(
      { workspaceId: 'ws-1', request: {} },
      expect.anything(),
    )
    expect(screen.queryByRole('button', { name: /reset to git/i })).not.toBeInTheDocument()
  })

  it('shows the action bar and editable inputs for a manual workspace', () => {
    currentWorkspace = workspace({ name: 'Manual Workspace', source: 'manual' })

    render(<WorkspaceConfigEditSection workspaceId="ws-1" />)

    expect(screen.queryByText('Synced from Git')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /save changes/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /delete workspace/i })).toBeInTheDocument()
    expect(screen.getByDisplayValue('Manual Workspace')).not.toBeDisabled()
  })

  it('deletes a manual workspace after confirmation', async () => {
    currentWorkspace = workspace({ name: 'Manual Workspace', source: 'manual' })
    const onDeleted = vi.fn()
    deleteMutate.mockImplementationOnce((_workspaceId, options) => {
      options.onSuccess()
    })
    const user = userEvent.setup()

    render(<WorkspaceConfigEditSection workspaceId="ws-1" onDeleted={onDeleted} />)
    await user.click(screen.getByRole('button', { name: /delete workspace/i }))
    await user.click(screen.getByRole('button', { name: /^delete$/i }))

    expect(deleteMutate).toHaveBeenCalledWith('ws-1', expect.anything())
    expect(onDeleted).toHaveBeenCalled()
  })

  it('deletes a git-origin workspace without navigating away', async () => {
    currentWorkspace = gitSourced()
    const onDeleted = vi.fn()
    deleteMutate.mockImplementationOnce((_workspaceId, options) => {
      options.onSuccess()
    })
    const user = userEvent.setup()

    render(<WorkspaceConfigEditSection workspaceId="ws-1" onDeleted={onDeleted} />)
    await user.click(screen.getByRole('button', { name: /delete workspace/i }))
    await user.click(screen.getByRole('button', { name: /^delete$/i }))

    expect(deleteMutate).toHaveBeenCalledWith('ws-1', expect.anything())
    expect(onDeleted).not.toHaveBeenCalled()
  })
})

describe('WorkspaceConfigEditSection (overridden)', () => {
  const overridden = (overrides: Partial<WorkspaceResponse> = {}) =>
    gitSourced({
      name: 'Overridden Workspace',
      source: 'manual',
      syncedFromGit: false,
      ...overrides,
    })

  it('is editable and warns that resetting replaces the local changes', () => {
    currentWorkspace = overridden()

    render(<WorkspaceConfigEditSection workspaceId="ws-1" />)

    expect(screen.getByText('Overridden — no longer synced from Git')).toBeInTheDocument()
    expect(screen.getByText(/replaces these local changes/i)).toBeInTheDocument()

    // Editing is unlocked again, and there is no "override" offer left.
    expect(screen.getByDisplayValue('Overridden Workspace')).not.toBeDisabled()
    expect(screen.getByRole('button', { name: /save changes/i })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /override to edit/i })).not.toBeInTheDocument()
  })

  it('resets back to git on demand', async () => {
    currentWorkspace = overridden()
    const user = userEvent.setup()

    render(<WorkspaceConfigEditSection workspaceId="ws-1" />)
    await user.click(screen.getByRole('button', { name: /reset to git/i }))

    expect(resetGitMutate).toHaveBeenCalledWith('ws-1', expect.anything())
  })

  it('leaves a workspace that was never in git without a provenance banner', () => {
    currentWorkspace = workspace({ source: 'manual' })

    render(<WorkspaceConfigEditSection workspaceId="ws-1" />)

    expect(screen.queryByRole('button', { name: /reset to git/i })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /override to edit/i })).not.toBeInTheDocument()
  })

  it('shows a deleted git-origin workspace as a restorable tombstone', async () => {
    currentWorkspace = overridden({
      deleted: true,
      name: 'Deleted Workspace',
    })
    const user = userEvent.setup()

    render(<WorkspaceConfigEditSection workspaceId="ws-1" />)

    expect(screen.getByText('Deleted override — not synced from Git')).toBeInTheDocument()
    expect(screen.getByText(/Git sync will not recreate it/i)).toBeInTheDocument()
    expect(screen.getByDisplayValue('Deleted Workspace')).toBeDisabled()
    expect(screen.queryByRole('button', { name: /delete workspace/i })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /save changes/i })).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /restore from git/i }))

    expect(resetGitMutate).toHaveBeenCalledWith('ws-1', expect.anything())
  })
})
