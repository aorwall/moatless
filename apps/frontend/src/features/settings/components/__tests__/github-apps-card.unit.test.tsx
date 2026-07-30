import { describe, expect, it, vi } from 'vitest'

import {
  useAdminGitHubApps,
  useCreateAdminGitHubBotUser,
  useRegisterAdminGitHubApp,
  useRemoveAdminGitHubApp,
  useRotateAdminGitHubAppKey,
} from '@/features/settings/hooks/use-admin-github-connection'
import { render, screen } from '@/test-utils/render'
import { GitHubAppsCard } from '../github-apps-card'

vi.mock('@/features/settings/hooks/use-admin-github-connection', () => ({
  useAdminGitHubApps: vi.fn(),
  useRegisterAdminGitHubApp: vi.fn(),
  useRotateAdminGitHubAppKey: vi.fn(),
  useRemoveAdminGitHubApp: vi.fn(),
  useCreateAdminGitHubBotUser: vi.fn(),
}))

const LOGIN_APP = {
  githubAppKey: 'default',
  name: 'Moatless',
  installUrl: 'https://github.com/apps/moatless/installations/new',
  installations: [],
  deploymentConfigured: true,
}

const BOT_APP = {
  githubAppKey: 'review-bot',
  name: 'Reviewer',
  installUrl: 'https://github.com/apps/reviewer/installations/new',
  installations: [{ installationId: 42, accountLogin: 'acme', accountType: 'Organization' }],
  deploymentConfigured: false,
  keyFingerprint: 'a1b2c3d4e5f60718',
  botLogin: 'reviewer[bot]',
}

function mockHooks(apps: unknown[]) {
  vi.mocked(useAdminGitHubApps).mockReturnValue({
    data: { apps },
    isLoading: false,
    error: null,
  } as unknown as ReturnType<typeof useAdminGitHubApps>)
  for (const hook of [
    useRegisterAdminGitHubApp,
    useRotateAdminGitHubAppKey,
    useRemoveAdminGitHubApp,
    useCreateAdminGitHubBotUser,
  ]) {
    vi.mocked(hook).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
      error: null,
    } as never)
  }
}

describe('GitHubAppsCard', () => {
  it('offers rotate and remove for a registered App, and its fingerprint', () => {
    mockHooks([BOT_APP])
    render(<GitHubAppsCard />)

    expect(screen.getByText('Reviewer')).toBeInTheDocument()
    expect(screen.getByText(/a1b2c3d4e5f60718/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /rotate key/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /remove/i })).toBeInTheDocument()
  })

  it('shows the deployment-configured App without offering to change it', () => {
    // Nothing on this page may make sign-in unreachable, so the login App is
    // listed and marked rather than hidden.
    mockHooks([LOGIN_APP])
    render(<GitHubAppsCard />)

    expect(screen.getByText('Configured by deployment')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /rotate key/i })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /remove/i })).not.toBeInTheDocument()
  })

  it('offers to create the bot user for an App that has none', () => {
    mockHooks([BOT_APP])
    render(<GitHubAppsCard />)

    expect(screen.getByRole('button', { name: /create bot user/i })).toBeInTheDocument()
  })

  it('does not re-offer the bot user once one exists', () => {
    mockHooks([{ ...BOT_APP, botUserId: 'b7b1f0a0-0000-4000-8000-000000000000' }])
    render(<GitHubAppsCard />)

    expect(screen.queryByRole('button', { name: /create bot user/i })).not.toBeInTheDocument()
    expect(screen.getByText(/bot reviewer\[bot\]/)).toBeInTheDocument()
  })

  it('says plainly when nothing is registered', () => {
    mockHooks([])
    render(<GitHubAppsCard />)

    expect(screen.getByText('No GitHub Apps are registered yet.')).toBeInTheDocument()
  })
})
