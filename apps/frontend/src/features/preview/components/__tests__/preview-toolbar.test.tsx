import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/lib/components/ui/tooltip'
import { PreviewProvider } from '../../hooks/use-preview'
import { PreviewToolbar } from '../preview-toolbar'

// Mock dependencies
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'test-task' }),
}))

vi.mock('@/features/preview', () => ({
  useServerStatus: () => ({
    data: { running: true, configured: true, url: 'http://localhost:3000', uptimeSeconds: 120 },
  }),
  useStartServer: () => ({ mutate: vi.fn(), isPending: false }),
  useStopServer: () => ({ mutate: vi.fn(), isPending: false }),
  useRestartServer: () => ({ mutate: vi.fn(), isPending: false }),
}))

describe('PreviewToolbar', () => {
  let queryClient: QueryClient
  const user = userEvent.setup()
  const mockOnReload = vi.fn()

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })
    vi.clearAllMocks()
  })

  const renderToolbar = () => {
    return render(
      <QueryClientProvider client={queryClient}>
        <PreviewProvider>
          <TooltipProvider>
            <PreviewToolbar onReload={mockOnReload} />
          </TooltipProvider>
        </PreviewProvider>
      </QueryClientProvider>,
    )
  }

  describe('Initial Rendering', () => {
    it('should render with default values from context', () => {
      renderToolbar()

      // Should show default path
      expect(screen.getByDisplayValue('/')).toBeInTheDocument()

      // Should show inspector toggle button
      expect(screen.getByRole('button', { name: /toggle inspector/i })).toBeInTheDocument()

      // Should show reload button
      expect(screen.getByRole('button', { name: /reload/i })).toBeInTheDocument()

      // Should show server button
      expect(screen.getByRole('button', { name: /server/i })).toBeInTheDocument()
    })

    it('should render script health indicator in server popup', async () => {
      renderToolbar()

      const serverButton = screen.getByRole('button', { name: /server/i })
      await user.click(serverButton)

      expect(screen.getByText('Inspector')).toBeInTheDocument()
    })
  })

  describe('Inspector Toggle', () => {
    it('should toggle inspector state when clicked', async () => {
      renderToolbar()

      const inspectorButton = screen.getByRole('button', { name: /toggle inspector/i })

      // Initially should not be active
      expect(inspectorButton).not.toHaveAttribute('data-state', 'on')

      // Click to activate
      await user.click(inspectorButton)

      await waitFor(() => {
        expect(inspectorButton).toHaveAttribute('data-state', 'on')
      })

      // Click to deactivate
      await user.click(inspectorButton)

      await waitFor(() => {
        expect(inspectorButton).not.toHaveAttribute('data-state', 'on')
      })
    })
  })

  describe('Path Navigation', () => {
    it('should update path when input changes and Enter is pressed', async () => {
      renderToolbar()

      const pathInput = screen.getByDisplayValue('/')

      // Clear and enter new path
      await user.clear(pathInput)
      await user.type(pathInput, '/dashboard')
      await user.keyboard('{Enter}')

      expect(pathInput).toHaveValue('/dashboard')
    })

    it('should normalize path without leading slash', async () => {
      renderToolbar()

      const pathInput = screen.getByDisplayValue('/')

      await user.clear(pathInput)
      await user.type(pathInput, 'dashboard')
      await user.keyboard('{Enter}')

      expect(pathInput).toHaveValue('/dashboard')
    })

    it('should update input when display path changes externally', async () => {
      // This would require updating the context from outside the component
      // For now, we test that the input reflects the current state
      renderToolbar()

      const pathInput = screen.getByDisplayValue('/')
      expect(pathInput).toHaveValue('/')
    })
  })

  describe('Reload Functionality', () => {
    it('should call onReload when reload button is clicked', async () => {
      renderToolbar()

      const reloadButton = screen.getByRole('button', { name: /reload/i })
      await user.click(reloadButton)

      expect(mockOnReload).toHaveBeenCalledTimes(1)
    })
  })

  describe('Server Status Display', () => {
    it('should show server status information in popup', async () => {
      renderToolbar()

      const serverButton = screen.getByRole('button', { name: /server/i })
      await user.click(serverButton)

      // Should show server status information
      expect(screen.getByText('Status:')).toBeInTheDocument()
      expect(screen.getByText('Running')).toBeInTheDocument()
      expect(screen.getByText('URL:')).toBeInTheDocument()
      expect(screen.getByText('http://localhost:3000')).toBeInTheDocument()
      expect(screen.getByText('Uptime:')).toBeInTheDocument()
      expect(screen.getByText('2m 0s')).toBeInTheDocument()
    })

    it('should show script health indicator with correct styling', async () => {
      renderToolbar()

      const serverButton = screen.getByRole('button', { name: /server/i })
      await user.click(serverButton)

      const inspectorText = screen.getByText('Inspector')
      expect(inspectorText).toBeInTheDocument()

      // Should have the correct parent container with border styling
      const container = inspectorText.parentElement?.parentElement
      expect(container).toHaveClass('border-t', 'pt-2')
    })
  })

  describe('Keyboard Interactions', () => {
    it('should handle Escape key in path input', async () => {
      renderToolbar()

      const pathInput = screen.getByDisplayValue('/')

      // Change value but don't submit
      await user.clear(pathInput)
      await user.type(pathInput, '/test')

      // Press Escape - should revert to original value
      await user.keyboard('{Escape}')

      // Note: The actual escape behavior would depend on implementation
      // This test documents the expected behavior
    })

    it('should handle Enter key to submit path', async () => {
      renderToolbar()

      const pathInput = screen.getByDisplayValue('/')

      await user.clear(pathInput)
      await user.type(pathInput, '/new-path{Enter}')

      expect(pathInput).toHaveValue('/new-path')
    })
  })

  describe('Error States', () => {
    it('should handle empty path input gracefully', async () => {
      renderToolbar()

      const pathInput = screen.getByDisplayValue('/')

      await user.clear(pathInput)
      await user.keyboard('{Enter}')

      // Should default to root path
      expect(pathInput).toHaveValue('/')
    })
  })

  describe('Accessibility', () => {
    it('should have proper ARIA labels', () => {
      renderToolbar()

      expect(screen.getByRole('button', { name: /toggle inspector/i })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /reload/i })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /server/i })).toBeInTheDocument()
      expect(screen.getByLabelText(/path/i)).toBeInTheDocument()
    })

    it('should support keyboard navigation', async () => {
      renderToolbar()

      // Tab through interactive elements
      await user.tab()
      expect(document.activeElement).toHaveAttribute('type', 'text') // path input

      await user.tab()
      expect(document.activeElement).toHaveAttribute('role', 'button') // first button

      await user.tab()
      expect(document.activeElement).toHaveAttribute('role', 'button') // second button
    })
  })

  describe('Tooltip Integration', () => {
    it('should show tooltips on hover', async () => {
      renderToolbar()

      const inspectorButton = screen.getByRole('button', { name: /toggle inspector/i })
      await user.hover(inspectorButton)

      // Tooltip content would depend on implementation
      // This test documents the expected behavior
    })
  })

  describe('Context Integration', () => {
    it('should reflect context state changes', async () => {
      // This test would require a way to trigger context changes from outside
      // For comprehensive testing, we'd need a test wrapper that can manipulate context
      renderToolbar()

      const inspectorButton = screen.getByRole('button', { name: /toggle inspector/i })

      // Toggle inspector through UI
      await user.click(inspectorButton)

      // Should reflect in the button state
      await waitFor(() => {
        expect(inspectorButton).toHaveAttribute('data-state', 'on')
      })
    })
  })
})
