import { describe, expect, it, vi } from 'vitest'

import {
  useAdminGitHubApps,
  useAdminGitHubInstallation,
  useRemoveAdminGitHubInstallation,
  useSaveAdminGitHubInstallation,
} from '@/features/settings/hooks/use-admin-github-connection'
import { render, screen } from '@/test-utils/render'
import { GitHubInstallationCard } from '../github-installation-card'

vi.mock('@/features/settings/hooks/use-admin-github-connection', () => ({
  useAdminGitHubApps: vi.fn(),
  useAdminGitHubInstallation: vi.fn(),
  useSaveAdminGitHubInstallation: vi.fn(),
  useRemoveAdminGitHubInstallation: vi.fn(),
}))

function mockHooks(configured: boolean) {
  vi.mocked(useAdminGitHubApps).mockReturnValue({
    data: {
      apps: [
        {
          githubAppKey: 'default',
          name: 'Moatless',
          installUrl: 'https://github.com/apps/moatless/installations/new',
          installations: [
            {
              installationId: 42,
              accountLogin: 'acme',
              accountType: 'Organization',
            },
          ],
        },
      ],
    },
    isLoading: false,
    isFetching: false,
    error: null,
    refetch: vi.fn(),
  } as unknown as ReturnType<typeof useAdminGitHubApps>)
  vi.mocked(useAdminGitHubInstallation).mockReturnValue({
    data: configured
      ? {
          configured: true,
          githubAppKey: 'default',
          installationId: 42,
          accountLogin: 'acme',
          accountType: 'Organization',
        }
      : { configured: false },
    isLoading: false,
    error: null,
  } as unknown as ReturnType<typeof useAdminGitHubInstallation>)
  vi.mocked(useSaveAdminGitHubInstallation).mockReturnValue({
    mutate: vi.fn(),
    isPending: false,
  } as unknown as ReturnType<typeof useSaveAdminGitHubInstallation>)
  vi.mocked(useRemoveAdminGitHubInstallation).mockReturnValue({
    mutate: vi.fn(),
    isPending: false,
  } as unknown as ReturnType<typeof useRemoveAdminGitHubInstallation>)
}

describe('GitHubInstallationCard', () => {
  it('shows the configured global installation and unbind semantics', () => {
    mockHooks(true)
    render(<GitHubInstallationCard />)

    expect(screen.getByText('Global GitHub access')).toBeInTheDocument()
    expect(screen.getByText('Installation configured')).toBeInTheDocument()
    expect(screen.getAllByText(/acme/).length).toBeGreaterThan(0)
    expect(screen.getByRole('button', { name: 'Unbind' })).toBeEnabled()
    expect(screen.getByText(/does not uninstall the App/)).toBeInTheDocument()
  })

  it('renders the same installation selector for a team', () => {
    mockHooks(false)
    render(<GitHubInstallationCard teamId="team-1" />)

    expect(useAdminGitHubInstallation).toHaveBeenCalledWith('team-1')
    expect(screen.getByText('Team GitHub access')).toBeInTheDocument()
    expect(screen.getByText('No installation bound')).toBeInTheDocument()
  })
})
