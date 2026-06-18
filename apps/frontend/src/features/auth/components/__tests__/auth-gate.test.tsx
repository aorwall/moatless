import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useAuth } from '@/features/auth/hooks/use-auth'
import { serverStatusStore } from '@/lib/api/server-status'
import { ServerDownGate } from '@/lib/components/server-down-gate'
import { AuthGate } from '../auth-gate'

// Render `<Navigate>` as an inspectable marker instead of performing a real
// router navigation.
vi.mock('@tanstack/react-router', () => ({
  Navigate: ({ to }: { to: string }) => <div data-testid="navigate" data-to={to} />,
}))

vi.mock('@/features/auth/hooks/use-auth', () => ({
  useAuth: vi.fn(),
}))

function mockAuth(overrides: Partial<ReturnType<typeof useAuth>>) {
  vi.mocked(useAuth).mockReturnValue({
    user: null,
    role: null,
    isAdmin: false,
    expiresAt: null,
    isAuthenticated: false,
    isLoading: false,
    isServerUnavailable: false,
    refetch: vi.fn(),
    providers: [],
    githubConnected: true,
    logout: vi.fn(),
    ...overrides,
  } as unknown as ReturnType<typeof useAuth>)
}

describe('AuthGate', () => {
  beforeEach(() => {
    serverStatusStore.setUnavailable(false)
    vi.clearAllMocks()
  })

  it('redirects to /login when unauthenticated and the server is reachable', () => {
    mockAuth({ isAuthenticated: false, isServerUnavailable: false })

    render(
      <AuthGate>
        <div>app</div>
      </AuthGate>,
    )

    expect(screen.getByTestId('navigate')).toHaveAttribute('data-to', '/login')
    expect(screen.queryByText('app')).not.toBeInTheDocument()
  })

  it('does NOT redirect when the session check fails with a 503', () => {
    // The /me query errored with a non-auth error; the global store has not
    // been set yet. The gate should render children, not bounce to /login.
    mockAuth({ isAuthenticated: false, isServerUnavailable: true })

    render(
      <AuthGate>
        <div>app</div>
      </AuthGate>,
    )

    expect(screen.queryByTestId('navigate')).not.toBeInTheDocument()
    expect(screen.getByText('app')).toBeInTheDocument()
  })

  it('shows the ServerDown screen and never redirects during downtime', () => {
    mockAuth({ isAuthenticated: false, isServerUnavailable: true })
    serverStatusStore.setUnavailable(true)

    render(
      <ServerDownGate>
        <AuthGate>
          <div>app</div>
        </AuthGate>
      </ServerDownGate>,
    )

    expect(screen.getByText('Server unavailable')).toBeInTheDocument()
    expect(screen.queryByText('app')).not.toBeInTheDocument()
    expect(screen.queryByTestId('navigate')).not.toBeInTheDocument()
  })

  it('renders the app when authenticated', () => {
    mockAuth({ isAuthenticated: true })

    render(
      <AuthGate>
        <div>app</div>
      </AuthGate>,
    )

    expect(screen.getByText('app')).toBeInTheDocument()
    expect(screen.queryByTestId('navigate')).not.toBeInTheDocument()
  })

  it('shows a loading state while the session is loading', () => {
    mockAuth({ isLoading: true })

    render(
      <AuthGate>
        <div>app</div>
      </AuthGate>,
    )

    expect(screen.getByText('Loading...')).toBeInTheDocument()
    expect(screen.queryByText('app')).not.toBeInTheDocument()
  })
})
