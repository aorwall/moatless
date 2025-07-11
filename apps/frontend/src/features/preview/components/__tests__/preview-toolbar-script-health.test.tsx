import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/lib/components/ui/tooltip'
import { usePreviewStore } from '../../stores/preview-store'
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

describe('PreviewToolbar - Script Health Integration', () => {
  let queryClient: QueryClient
  const user = userEvent.setup()

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })

    // Reset store
    act(() => {
      usePreviewStore.getState().reset()
    })
  })

  const renderToolbar = () => {
    return render(
      <QueryClientProvider client={queryClient}>
        <TooltipProvider>
          <PreviewToolbar onReload={vi.fn()} />
        </TooltipProvider>
      </QueryClientProvider>,
    )
  }

  describe('Script Health Indicator Display', () => {
    it('shows script health indicator in server control popup', async () => {
      renderToolbar()

      // Open server control popup - find by class name (server status button has the status color class)
      const serverButton = document.querySelector('.text-status-success')?.closest('button')
      expect(serverButton).toBeTruthy()
      await user.click(serverButton!)

      // Should show script health indicator
      expect(screen.getByText('Inspector')).toBeInTheDocument()
    })

    it('displays correct status for not-injected', async () => {
      act(() => {
        usePreviewStore.getState().setScriptHealthStatus('not-injected')
      })

      renderToolbar()

      // Open server control popup - find by class name (server status button has the status color class)
      const serverButton = document.querySelector('.text-status-success')?.closest('button')
      expect(serverButton).toBeTruthy()
      await user.click(serverButton!)

      // Check for muted color class (not-injected status)
      const indicator = screen.getByText('Inspector').parentElement
      expect(indicator?.querySelector('.text-muted-foreground')).toBeInTheDocument()
    })

    it('displays correct status for injected-not-reachable', async () => {
      act(() => {
        usePreviewStore.getState().setScriptHealthStatus('injected-not-reachable')
      })

      renderToolbar()

      // Open server control popup - find by class name (server status button has the status color class)
      const serverButton = document.querySelector('.text-status-success')?.closest('button')
      expect(serverButton).toBeTruthy()
      await user.click(serverButton!)

      // Check for yellow color class (warning status)
      const indicator = screen.getByText('Inspector').parentElement
      expect(indicator?.querySelector('.text-yellow-500')).toBeInTheDocument()
    })

    it('displays correct status for injected-reachable', async () => {
      act(() => {
        usePreviewStore.getState().setScriptHealthStatus('injected-reachable')
      })

      renderToolbar()

      // Open server control popup - find by class name (server status button has the status color class)
      const serverButton = document.querySelector('.text-status-success')?.closest('button')
      expect(serverButton).toBeTruthy()
      await user.click(serverButton!)

      // Check for green color class (success status)
      const indicator = screen.getByText('Inspector').parentElement
      expect(indicator?.querySelector('.text-green-500')).toBeInTheDocument()
    })
  })

  describe('Server Control Popup Layout', () => {
    it('shows script health indicator below server information', async () => {
      renderToolbar()

      // Open server control popup - find by class name (server status button has the status color class)
      const serverButton = document.querySelector('.text-status-success')?.closest('button')
      expect(serverButton).toBeTruthy()
      await user.click(serverButton!)

      // Get server control section - the popup doesn't have dialog role with our mocked components
      // Instead, check for the content directly

      // Should contain server status
      expect(screen.getByText('Status:')).toBeInTheDocument()
      expect(screen.getByText('Running')).toBeInTheDocument()

      // Should contain URL
      expect(screen.getByText('URL:')).toBeInTheDocument()
      expect(screen.getByText('http://localhost:3000')).toBeInTheDocument()

      // Should contain uptime
      expect(screen.getByText('Uptime:')).toBeInTheDocument()
      expect(screen.getByText('2m 0s')).toBeInTheDocument()

      // Should contain script health indicator
      expect(screen.getByText('Inspector')).toBeInTheDocument()
    })

    it('shows script health indicator with border separator', async () => {
      renderToolbar()

      // Open server control popup - find by class name (server status button has the status color class)
      const serverButton = document.querySelector('.text-status-success')?.closest('button')
      expect(serverButton).toBeTruthy()
      await user.click(serverButton!)

      // Check for border-t class on the container
      const indicator = screen.getByText('Inspector').parentElement?.parentElement
      expect(indicator?.classList.contains('border-t')).toBe(true)
      expect(indicator?.classList.contains('pt-2')).toBe(true)
    })
  })

  describe('Dynamic Updates', () => {
    it('updates script health indicator when status changes', async () => {
      renderToolbar()

      // Open server control popup - find by class name (server status button has the status color class)
      const serverButton = document.querySelector('.text-status-success')?.closest('button')
      expect(serverButton).toBeTruthy()
      await user.click(serverButton!)

      // Initial status
      expect(
        screen.getByText('Inspector').parentElement?.querySelector('.text-muted-foreground'),
      ).toBeInTheDocument()

      // Update status
      act(() => {
        usePreviewStore.getState().setScriptHealthStatus('injected-reachable')
      })

      // Should update to show green indicator
      expect(
        screen.getByText('Inspector').parentElement?.querySelector('.text-green-500'),
      ).toBeInTheDocument()
    })

    it('maintains script health display when popup is reopened', async () => {
      // Set initial status
      act(() => {
        usePreviewStore.getState().setScriptHealthStatus('injected-reachable')
      })

      renderToolbar()

      // Open popup
      const serverButton = screen.getByRole('button', { name: /server/i })
      await user.click(serverButton)

      // Verify status
      expect(
        screen.getByText('Inspector').parentElement?.querySelector('.text-green-500'),
      ).toBeInTheDocument()

      // Close popup by clicking outside
      await user.click(document.body)

      // Wait a moment for popup to close
      await new Promise((resolve) => setTimeout(resolve, 100))

      // Reopen popup - find server button again
      const serverButtonAgain = document.querySelector('.text-status-success')?.closest('button')
      expect(serverButtonAgain).toBeTruthy()
      await user.click(serverButtonAgain!)

      // Status should be maintained
      expect(
        screen.getByText('Inspector').parentElement?.querySelector('.text-green-500'),
      ).toBeInTheDocument()
    })
  })

  describe('Tooltip Integration', () => {
    it('shows script health tooltip on hover', async () => {
      act(() => {
        usePreviewStore.getState().setScriptHealthStatus('injected-reachable')
      })

      renderToolbar()

      // Open server control popup - find by class name (server status button has the status color class)
      const serverButton = document.querySelector('.text-status-success')?.closest('button')
      expect(serverButton).toBeTruthy()
      await user.click(serverButton!)

      // Hover over script health indicator
      const indicator = screen.getByText('Inspector').parentElement
      await user.hover(indicator!)

      // Should show tooltip
      await screen.findByText('Script Active')
      expect(
        screen.getByText('Inspector script is injected and actively communicating'),
      ).toBeInTheDocument()
    })
  })
})
