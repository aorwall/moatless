import { render, screen } from '@testing-library/react'
import { createMemoryRouter, RouterProvider } from '@tanstack/react-router'
import { describe, expect, it, vi } from 'vitest'
import { AppSidebar } from '../app-sidebar'

// Mock child components
vi.mock('../sidebar-tasks', () => ({
  SidebarTasks: () => <div data-testid="sidebar-tasks">Tasks</div>,
}))

vi.mock('../sidebar-repositories', () => ({
  SidebarRepositories: () => <div data-testid="sidebar-repositories">Repositories</div>,
}))

vi.mock('../sidebar-commands', () => ({
  SidebarCommands: () => <div data-testid="sidebar-commands">Commands</div>,
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

describe('AppSidebar', () => {
  const createTestRouter = () => {
    return createMemoryRouter({
      routes: [
        {
          path: '/',
          component: AppSidebar,
        },
        {
          path: '/settings',
          component: () => <div>Settings</div>,
        },
      ],
      initialEntries: ['/'],
    })
  }

  it('should render all main sections', () => {
    const router = createTestRouter()
    render(<RouterProvider router={router} />)

    // Check main structure
    expect(screen.getByTestId('sidebar')).toBeInTheDocument()
    expect(screen.getByTestId('sidebar-header')).toBeInTheDocument()
    expect(screen.getByTestId('sidebar-content')).toBeInTheDocument()
    expect(screen.getByTestId('sidebar-footer')).toBeInTheDocument()
  })

  it('should display application title', () => {
    const router = createTestRouter()
    render(<RouterProvider router={router} />)

    expect(screen.getByText('Moatless Vibe')).toBeInTheDocument()
  })

  it('should render navigation links', () => {
    const router = createTestRouter()
    render(<RouterProvider router={router} />)

    // Check main navigation items
    const dashboardLink = screen.getByRole('link', { name: /dashboard/i })
    expect(dashboardLink).toBeInTheDocument()
    expect(dashboardLink).toHaveAttribute('href', '/')

    const settingsLink = screen.getByRole('link', { name: /settings/i })
    expect(settingsLink).toBeInTheDocument()
    expect(settingsLink).toHaveAttribute('href', '/settings')
  })

  it('should render all sidebar sections', () => {
    const router = createTestRouter()
    render(<RouterProvider router={router} />)

    // Check that all sections are rendered
    expect(screen.getByTestId('sidebar-tasks')).toBeInTheDocument()
    expect(screen.getByTestId('sidebar-repositories')).toBeInTheDocument()
    expect(screen.getByTestId('sidebar-commands')).toBeInTheDocument()
  })

  it('should display version in footer', () => {
    const router = createTestRouter()
    render(<RouterProvider router={router} />)

    expect(screen.getByText('v1.0.0')).toBeInTheDocument()
  })
})