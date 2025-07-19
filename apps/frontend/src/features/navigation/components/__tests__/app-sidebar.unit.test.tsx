import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { AppSidebar } from '../app-sidebar'

// Mock child components
vi.mock('../sidebar-tasks', () => ({
  SidebarTasks: () => <div data-testid="sidebar-tasks">Tasks</div>,
}))

vi.mock('@/features/repositories', () => ({
  RepositoriesConfigurationDialog: () => null,
}))

// Mock UI components
vi.mock('@/lib/components/ui/sidebar', () => ({
  Sidebar: ({ children }: any) => <div data-testid="sidebar">{children}</div>,
  SidebarContent: ({ children }: any) => <div data-testid="sidebar-content">{children}</div>,
  SidebarFooter: ({ children }: any) => <div data-testid="sidebar-footer">{children}</div>,
  SidebarGroup: ({ children }: any) => <div data-testid="sidebar-group">{children}</div>,
  SidebarGroupContent: ({ children }: any) => (
    <div data-testid="sidebar-group-content">{children}</div>
  ),
  SidebarHeader: ({ children }: any) => <div data-testid="sidebar-header">{children}</div>,
  SidebarMenu: ({ children }: any) => <ul data-testid="sidebar-menu">{children}</ul>,
  SidebarMenuButton: ({ children, asChild, ...props }: any) => {
    if (asChild) {
      return children
    }
    return <button {...props}>{children}</button>
  },
  SidebarMenuItem: ({ children }: any) => <li data-testid="sidebar-menu-item">{children}</li>,
}))

// Mock router hooks
vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, to, ...props }: any) => (
    <a href={to} {...props}>
      {children}
    </a>
  ),
  useNavigate: () => vi.fn(),
}))

// Mock global event stream
vi.mock('@/features/events/contexts/global-event-stream-provider', () => ({
  useGlobalEventStream: () => ({
    isConnected: true,
    connectionError: null,
  }),
}))

// Create a wrapper component with providers
const wrapper = ({ children }: { children: React.ReactNode }) => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  })
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
}

const renderWithProviders = (ui: React.ReactElement) => {
  return render(ui, { wrapper })
}

describe('AppSidebar', () => {
  it('should render all main sections', () => {
    renderWithProviders(<AppSidebar />)

    // Check main structure
    expect(screen.getByTestId('sidebar')).toBeInTheDocument()
    expect(screen.getByTestId('sidebar-header')).toBeInTheDocument()
    expect(screen.getByTestId('sidebar-content')).toBeInTheDocument()
    expect(screen.getByTestId('sidebar-footer')).toBeInTheDocument()
  })

  it('should display application title', () => {
    renderWithProviders(<AppSidebar />)

    expect(screen.getByText('Moatless Vibe')).toBeInTheDocument()
  })

  it('should render navigation links', () => {
    renderWithProviders(<AppSidebar />)

    // Check main navigation items
    const dashboardLink = screen.getByRole('link', { name: /dashboard/i })
    expect(dashboardLink).toBeInTheDocument()
    expect(dashboardLink).toHaveAttribute('href', '/')

    const settingsLink = screen.getByRole('link', { name: /settings/i })
    expect(settingsLink).toBeInTheDocument()
    expect(settingsLink).toHaveAttribute('href', '/settings')
  })

  it('should render all sidebar sections', () => {
    renderWithProviders(<AppSidebar />)

    // Check that all sections are rendered
    expect(screen.getByTestId('sidebar-tasks')).toBeInTheDocument()
  })

  it('should display version in footer', () => {
    renderWithProviders(<AppSidebar />)

    expect(screen.getByText('v1.0.0')).toBeInTheDocument()
  })
})
