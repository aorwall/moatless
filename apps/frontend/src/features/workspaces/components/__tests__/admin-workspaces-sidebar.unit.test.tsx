import type { WorkspaceResponse } from '@moatless/api-client'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@/test-utils/render'
import { AdminWorkspacesSidebar } from '../admin-workspaces-sidebar'

let workspaces: WorkspaceResponse[] = []

vi.mock('@tanstack/react-router', () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/admin/workspaces">{children}</a>,
}))

vi.mock('@/features/workspaces/hooks/use-workspaces', () => ({
  useWorkspacesList: () => ({ data: workspaces, isLoading: false, error: null }),
}))

function workspace(overrides: Partial<WorkspaceResponse>): WorkspaceResponse {
  return {
    id: 'ws-active',
    name: 'Active Workspace',
    kind: 'single',
    scope: 'user',
    repos: [],
    source: 'manual',
    ...overrides,
  }
}

describe('AdminWorkspacesSidebar', () => {
  beforeEach(() => {
    workspaces = [
      workspace({ id: 'ws-active', name: 'Active Workspace' }),
      workspace({
        id: 'ws-deleted',
        name: 'Deleted Workspace',
        deleted: true,
        sourceRepositoryId: 'repo-1',
        sourceKey: 'api',
      }),
    ]
  })

  it('hides deleted git overrides by default and reveals them with the filter', async () => {
    const user = userEvent.setup()

    render(<AdminWorkspacesSidebar />)

    expect(screen.getByText('Active Workspace')).toBeInTheDocument()
    expect(screen.queryByText('Deleted Workspace')).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /show deleted git overrides/i }))

    expect(screen.getByText('Deleted Workspace')).toBeInTheDocument()
    expect(screen.getByText('Deleted Git override')).toBeInTheDocument()
  })
})
