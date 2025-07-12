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

describe('Preview Components Integration with Context', () => {
  let queryClient: QueryClient
  const user = userEvent.setup()

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })
  })

  const renderWithProviders = (component: React.ReactElement) => {
    return render(
      <QueryClientProvider client={queryClient}>
        <PreviewProvider>
          <TooltipProvider>{component}</TooltipProvider>
        </PreviewProvider>
      </QueryClientProvider>,
    )
  }

  describe('PreviewToolbar Integration', () => {
    it('should render with initial context state', () => {
      renderWithProviders(<PreviewToolbar onReload={vi.fn()} />)

      // Should show the default URL
      expect(screen.getByDisplayValue('/')).toBeInTheDocument()
    })

    it('should display script health indicator', async () => {
      renderWithProviders(<PreviewToolbar onReload={vi.fn()} />)

      // Find server button and click it to open the popup
      const serverButton = screen.getByRole('button', { name: /server/i })
      await user.click(serverButton)

      // Should show script health indicator
      expect(screen.getByText('Inspector')).toBeInTheDocument()
    })

    it('should toggle inspector when clicking inspector button', async () => {
      renderWithProviders(<PreviewToolbar onReload={vi.fn()} />)

      // Find inspector toggle button
      const inspectorButton = screen.getByRole('button', { name: /toggle inspector/i })
      expect(inspectorButton).toBeInTheDocument()

      // Initially not active (should have default styling)
      expect(inspectorButton).not.toHaveAttribute('data-state', 'on')

      // Click to activate
      await user.click(inspectorButton)

      // Should now be active
      await waitFor(() => {
        expect(inspectorButton).toHaveAttribute('data-state', 'on')
      })
    })

    it('should update path input when navigating', async () => {
      renderWithProviders(<PreviewToolbar onReload={vi.fn()} />)

      const pathInput = screen.getByDisplayValue('/')

      // Change the path
      await user.clear(pathInput)
      await user.type(pathInput, '/dashboard')
      await user.keyboard('{Enter}')

      // Should update the input value
      expect(pathInput).toHaveValue('/dashboard')
    })
  })

  describe('Context State Management in Components', () => {
    it('should reflect context changes across component re-renders', async () => {
      const TestComponent = () => {
        return (
          <div>
            <PreviewToolbar onReload={vi.fn()} />
            <div data-testid="inspector-status">
              {/* This will be updated by context changes */}
            </div>
          </div>
        )
      }

      renderWithProviders(<TestComponent />)

      const inspectorButton = screen.getByRole('button', { name: /toggle inspector/i })

      // Initial state
      expect(inspectorButton).not.toHaveAttribute('data-state', 'on')

      // Toggle inspector
      await user.click(inspectorButton)

      // Should reflect the change
      await waitFor(() => {
        expect(inspectorButton).toHaveAttribute('data-state', 'on')
      })
    })
  })
})
