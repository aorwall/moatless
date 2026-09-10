import { fireEvent, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import {
  useDisconnectForgejo,
  useForgejoConfig,
  useForgejoInstances,
  useRemoveForgejoPatOverride,
  useSaveForgejoPAT,
} from '@/features/settings/hooks/use-forgejo-connection'
import { render } from '@/test-utils/render'
import { ForgejoConfigurationSection } from '../forgejo-configuration-section'

vi.mock('sonner', () => ({ toast: { error: vi.fn(), info: vi.fn(), success: vi.fn() } }))

vi.mock('@/features/settings/hooks/use-forgejo-connection', () => ({
  useForgejoInstances: vi.fn(),
  useForgejoConfig: vi.fn(),
  useDisconnectForgejo: vi.fn(),
  useRemoveForgejoPatOverride: vi.fn(),
  useSaveForgejoPAT: vi.fn(),
}))

const OAUTH_INSTANCE = {
  host: 'git.example.com',
  connected: false,
  hasPatOverride: false,
  oauthAvailable: true,
}

const TOKEN_ONLY_INSTANCE = {
  host: 'code.internal',
  connected: false,
  hasPatOverride: false,
  oauthAvailable: false,
}

function mockHooks(list: { data?: unknown; isLoading?: boolean; error?: unknown }) {
  vi.mocked(useForgejoInstances).mockReturnValue({
    data: list.data,
    isLoading: list.isLoading ?? false,
    error: list.error ?? null,
  } as unknown as ReturnType<typeof useForgejoInstances>)

  // Any host the list does not carry resolves to a status of its own, so a
  // failed dedupe shows as a second card rather than as nothing at all.
  vi.mocked(useForgejoConfig).mockImplementation(
    (host: string) =>
      ({
        data: host === '' ? undefined : { ...TOKEN_ONLY_INSTANCE, host },
        isLoading: false,
        error: null,
      }) as unknown as ReturnType<typeof useForgejoConfig>,
  )

  // `mutate` runs its `onSuccess`, so a test reaches what the component does
  // after a save instead of stopping at the call.
  for (const hook of [useDisconnectForgejo, useRemoveForgejoPatOverride, useSaveForgejoPAT]) {
    vi.mocked(hook).mockReturnValue({
      mutate: vi.fn((_variables: unknown, options?: { onSuccess?: () => void }) =>
        options?.onSuccess?.(),
      ),
      isPending: false,
      error: null,
    } as never)
  }
}

describe('ForgejoConfigurationSection', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('lists a registered instance and offers to connect it', () => {
    mockHooks({ data: { instances: [OAUTH_INSTANCE] } })
    render(<ForgejoConfigurationSection />)

    expect(screen.getByText('git.example.com')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /connect$/i })).toBeInTheDocument()
  })

  it('offers a token instead when the instance has no OAuth application', () => {
    mockHooks({ data: { instances: [TOKEN_ONLY_INSTANCE] } })
    render(<ForgejoConfigurationSection />)

    expect(screen.queryByRole('button', { name: /connect$/i })).not.toBeInTheDocument()
    expect(screen.getByLabelText('Personal Access Token')).toBeInTheDocument()
  })

  it('says plainly when no instance is registered', () => {
    mockHooks({ data: { instances: [] } })
    render(<ForgejoConfigurationSection />)

    expect(screen.getByText('No Forgejo instance registered')).toBeInTheDocument()
  })

  it('spins while the list loads, rather than claiming the list is empty', () => {
    mockHooks({ isLoading: true })
    render(<ForgejoConfigurationSection />)

    expect(screen.getByTestId('loading-spinner')).toBeInTheDocument()
    expect(screen.queryByText('No Forgejo instance registered')).not.toBeInTheDocument()
  })

  it('reports a failed read instead of an empty list', () => {
    mockHooks({ error: new Error('boom') })
    render(<ForgejoConfigurationSection />)

    expect(screen.getByText(/failed to load the forgejo instances/i)).toBeInTheDocument()
    expect(screen.queryByText('No Forgejo instance registered')).not.toBeInTheDocument()
  })

  it('closes the token form once the token is saved', () => {
    mockHooks({ data: { instances: [OAUTH_INSTANCE] } })
    render(<ForgejoConfigurationSection />)

    fireEvent.click(screen.getByRole('button', { name: 'Use a personal access token' }))
    fireEvent.change(screen.getByLabelText('Personal Access Token'), {
      target: { value: 'forgejo_pat' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))

    expect(screen.queryByLabelText('Personal Access Token')).not.toBeInTheDocument()
  })

  it('lists a host once when the typed form differs only in what the server drops', () => {
    mockHooks({ data: { instances: [OAUTH_INSTANCE] } })
    render(<ForgejoConfigurationSection />)

    fireEvent.change(screen.getByPlaceholderText('git.example.com'), {
      target: { value: 'https://Git.Example.com./owner/repo?ref=main#top' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Check' }))

    // A regex, because the card the dedupe has to prevent is headed by the
    // typed spelling rather than by the listed one.
    expect(screen.getAllByText(/^git\.example\.com\.?$/)).toHaveLength(1)
  })
})
