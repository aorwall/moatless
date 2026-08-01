import type { AdapterAppSummary } from '@moatless/api-client'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  useAdapterApps,
  useDeleteAdapterApp,
  useSaveAdapterApp,
} from '@/features/integrations/hooks/use-adapter-apps'
import { render, screen } from '@/test-utils/render'
import { AdminAdapterAppsPage } from '../admin-adapter-apps-page'

vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, to }: { children: ReactNode; to: string }) => <a href={to}>{children}</a>,
}))

vi.mock('@/features/integrations/hooks/use-adapter-apps', () => ({
  useAdapterApps: vi.fn(),
  useSaveAdapterApp: vi.fn(),
  useDeleteAdapterApp: vi.fn(),
}))

const saveMutate = vi.fn()
const deleteMutate = vi.fn()

function adapterApp(overrides: Partial<AdapterAppSummary>): AdapterAppSummary {
  return {
    adapterKind: 'slack',
    appKey: 'default',
    displayName: 'Slack',
    config: {},
    revision: 1,
    secretFingerprints: {},
    updatedAt: '2026-08-01T04:00:00Z',
    ...overrides,
  }
}

function mockHooks(apps: AdapterAppSummary[]) {
  vi.mocked(useAdapterApps).mockReturnValue({
    data: apps,
    isLoading: false,
    error: null,
  } as unknown as ReturnType<typeof useAdapterApps>)
  vi.mocked(useSaveAdapterApp).mockReturnValue({
    mutate: saveMutate,
    isPending: false,
    error: null,
  } as unknown as ReturnType<typeof useSaveAdapterApp>)
  vi.mocked(useDeleteAdapterApp).mockReturnValue({
    mutate: deleteMutate,
    isPending: false,
    error: null,
  } as unknown as ReturnType<typeof useDeleteAdapterApp>)
}

describe('AdminAdapterAppsPage', () => {
  beforeEach(() => {
    saveMutate.mockReset()
    deleteMutate.mockReset()
  })

  it('renders configured apps without exposing secret values', () => {
    mockHooks([
      adapterApp({
        adapterKind: 'slack',
        displayName: 'Slack',
        config: { bot_user_id: 'U123' },
        secretFingerprints: {
          bot_token: 'abc123',
          signing_secret: 'def456',
        },
      }),
      adapterApp({
        adapterKind: 'github',
        displayName: 'Reviewer',
        config: { app_id: '42', bot_login: 'reviewer[bot]' },
        secretFingerprints: { private_key: 'fedcba' },
      }),
    ])

    render(<AdminAdapterAppsPage />)

    expect(screen.getByText('Adapter Apps')).toBeInTheDocument()
    expect(screen.getAllByText('Slack').length).toBeGreaterThan(0)
    expect(screen.getAllByText('GitHub').length).toBeGreaterThan(0)
    expect(screen.getByText(/bot_token: abc123/)).toBeInTheDocument()
    expect(screen.getByText(/private_key: fedcba/)).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /manage/i })).toHaveAttribute(
      'href',
      '/admin/integrations/github',
    )
    expect(screen.queryByDisplayValue('abc123')).not.toBeInTheDocument()
  })

  it('submits Slack edits with replacement secrets and existing public config', async () => {
    const user = userEvent.setup()
    mockHooks([
      adapterApp({
        adapterKind: 'slack',
        config: { bot_user_id: 'U123' },
        secretFingerprints: {
          bot_token: 'old-token-fingerprint',
          signing_secret: 'old-secret-fingerprint',
        },
      }),
    ])

    render(<AdminAdapterAppsPage />)

    await user.click(screen.getByRole('button', { name: /edit/i }))

    expect(screen.getByLabelText('Bot user ID')).toHaveValue('U123')
    expect(screen.getByLabelText('Bot token')).toHaveValue('')
    expect(screen.getByLabelText('Signing secret')).toHaveValue('')

    await user.type(screen.getByLabelText('Bot token'), 'xoxb-new')
    await user.type(screen.getByLabelText('Signing secret'), 'slack-signing-secret')
    await user.click(screen.getByRole('button', { name: 'Save' }))

    expect(saveMutate).toHaveBeenCalledWith(
      {
        adapterKind: 'slack',
        appKey: 'default',
        data: {
          adapterKind: 'slack',
          botToken: 'xoxb-new',
          signingSecret: 'slack-signing-secret',
          botUserId: 'U123',
        },
      },
      expect.any(Object),
    )
  })

  it('keeps Telegram save disabled until a replacement token is entered', async () => {
    const user = userEvent.setup()
    mockHooks([
      adapterApp({
        adapterKind: 'telegram',
        displayName: 'Telegram',
        config: { mode: 'polling' },
        secretFingerprints: { bot_token: 'telegram-token-fingerprint' },
      }),
    ])

    render(<AdminAdapterAppsPage />)

    await user.click(screen.getByRole('button', { name: /edit/i }))

    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled()
    expect(screen.getByText(/backend restarts/i)).toBeInTheDocument()

    await user.type(screen.getByLabelText('Bot token'), 'telegram-token')

    expect(screen.getByRole('button', { name: 'Save' })).toBeEnabled()
  })
})
