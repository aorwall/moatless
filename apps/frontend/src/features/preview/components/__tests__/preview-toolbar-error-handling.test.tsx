import type { ServerStatus } from '@moatless/shared/types/workspace'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen } from '@testing-library/react'
import { toast } from 'sonner'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { PreviewToolbar } from '../preview-toolbar'

// Mock dependencies
vi.mock('@tanstack/react-router', () => ({
  useParams: vi.fn(() => ({ taskId: 'test-task-id' })),
}))

vi.mock('sonner', () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}))

vi.mock('@/features/preview', () => ({
  useServerStatus: vi.fn(),
  useStartServer: vi.fn(),
  useStopServer: vi.fn(),
  useRestartServer: vi.fn(),
}))

vi.mock('../stores/preview-store', () => ({
  usePreviewStore: vi.fn(() => ({
    config: { url: 'http://localhost:3000' },
    currentPath: '/',
    displayPath: '/',
    isInspectorActive: false,
    scriptHealthStatus: 'not-injected',
    navigateToPath: vi.fn(),
    toggleInspector: vi.fn(),
  })),
}))

// Mock UI components
vi.mock('@/lib/components/ui/button', () => ({
  Button: ({ children, onClick, disabled, ...props }: any) => (
    <button onClick={onClick} disabled={disabled} {...props}>
      {children}
    </button>
  ),
}))

vi.mock('@/lib/components/ui/input', () => ({
  Input: (props: any) => <input {...props} />,
}))

vi.mock('@/lib/components/ui/popover', () => ({
  Popover: ({ children }: any) => <div data-testid="popover">{children}</div>,
  PopoverContent: ({ children }: any) => <div data-testid="popover-content">{children}</div>,
  PopoverTrigger: ({ children }: any) => children,
}))

vi.mock('@/lib/components/ui/tooltip', () => ({
  Tooltip: ({ children }: any) => children,
  TooltipContent: ({ children }: any) => <div>{children}</div>,
  TooltipTrigger: ({ children }: any) => children,
}))

vi.mock('../script-health-indicator', () => ({
  ScriptHealthIndicator: () => <div data-testid="script-health-indicator">Health Status</div>,
}))

describe('PreviewToolbar Error Handling', () => {
  let queryClient: QueryClient
  let mockUseServerStatus: any
  let mockUseStartServer: any
  let mockUseStopServer: any
  let mockUseRestartServer: any

  beforeEach(async () => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })

    // Get the mocked modules
    const previewModule = await import('@/features/preview')
    mockUseServerStatus = previewModule.useServerStatus
    mockUseStartServer = previewModule.useStartServer
    mockUseStopServer = previewModule.useStopServer
    mockUseRestartServer = previewModule.useRestartServer

    // Reset all mocks
    vi.clearAllMocks()
  })

  afterEach(() => {
    queryClient.clear()
  })

  const renderWithQuery = (component: React.ReactElement) => {
    return render(<QueryClientProvider client={queryClient}>{component}</QueryClientProvider>)
  }

  it('should display failed status when server has error', () => {
    const serverStatusWithError: ServerStatus = {
      running: false,
      configured: true,
      uptimeSeconds: 0,
      error: 'Failed to start server: Command "start:frontend" not found',
      logs: 'ERR_PNPM_RECURSIVE_EXEC_FIRST_FAIL  Command "start:frontend" not found\n\nDid you mean "pnpm start:production"?',
    }

    mockUseServerStatus.mockReturnValue({ data: serverStatusWithError })
    mockUseStartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseStopServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseRestartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })

    renderWithQuery(<PreviewToolbar onReload={vi.fn()} />)

    // Check that "Failed" status is displayed (since popover is always shown in mock)
    expect(screen.getByText('Failed')).toBeInTheDocument()
  })

  it('should display error details in server control popup', () => {
    const serverStatusWithError: ServerStatus = {
      running: false,
      configured: true,
      uptimeSeconds: 0,
      error: 'Failed to start server: Command "start:frontend" not found',
      logs: 'ERR_PNPM_RECURSIVE_EXEC_FIRST_FAIL  Command "start:frontend" not found\n\nDid you mean "pnpm start:production"?',
    }

    mockUseServerStatus.mockReturnValue({ data: serverStatusWithError })
    mockUseStartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseStopServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseRestartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })

    renderWithQuery(<PreviewToolbar onReload={vi.fn()} />)

    // Check that error details are displayed
    expect(screen.getByText('Error:')).toBeInTheDocument()
    expect(screen.getAllByText(/start:frontend.*not found/)).toHaveLength(2) // Appears in both error and logs

    // Check that logs are displayed
    expect(screen.getByText('Logs:')).toBeInTheDocument()
    expect(screen.getByText(/ERR_PNPM_RECURSIVE_EXEC_FIRST_FAIL/)).toBeInTheDocument()
    expect(screen.getByText(/Did you mean.*start:production/)).toBeInTheDocument()
  })

  it('should show error toast when server start fails', async () => {
    const mockMutate = vi.fn()
    const startServerMock = {
      mutate: mockMutate,
      isPending: false,
    }

    mockUseServerStatus.mockReturnValue({
      data: { running: false, configured: true, uptimeSeconds: 0 },
    })
    mockUseStartServer.mockReturnValue(startServerMock)
    mockUseStopServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseRestartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })

    renderWithQuery(<PreviewToolbar onReload={vi.fn()} />)

    // Find and click the start server button
    const startButton = screen.getByText('Start Server')
    fireEvent.click(startButton)

    // Verify that the mutation was called
    expect(mockMutate).toHaveBeenCalledWith('test-task-id')

    // Simulate the onSuccess callback being called with error state
    const onSuccessCallback = mockUseStartServer.mock.calls[0][0].onSuccess
    const failedServerStatus: ServerStatus = {
      running: false,
      configured: true,
      uptimeSeconds: 0,
      error: 'Failed to start server: Command not found',
    }

    onSuccessCallback(failedServerStatus)

    // Verify error toast was called
    expect(toast.error).toHaveBeenCalledWith('Failed to start preview environment server', {
      description: 'Failed to start server: Command not found',
    })
  })

  it('should show success toast when server starts successfully', async () => {
    const mockMutate = vi.fn()
    const startServerMock = {
      mutate: mockMutate,
      isPending: false,
    }

    mockUseServerStatus.mockReturnValue({
      data: { running: false, configured: true, uptimeSeconds: 0 },
    })
    mockUseStartServer.mockReturnValue(startServerMock)
    mockUseStopServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseRestartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })

    renderWithQuery(<PreviewToolbar onReload={vi.fn()} />)

    // Find and click the start server button
    const startButton = screen.getByText('Start Server')
    fireEvent.click(startButton)

    // Simulate the onSuccess callback being called with success state
    const onSuccessCallback = mockUseStartServer.mock.calls[0][0].onSuccess
    const successServerStatus: ServerStatus = {
      running: true,
      configured: true,
      uptimeSeconds: 0,
      url: 'http://localhost:3001',
      pid: 12345,
    }

    onSuccessCallback(successServerStatus)

    // Verify success toast was called
    expect(toast.success).toHaveBeenCalledWith('Preview environment server started successfully')
  })

  it('should show error toast when server operation fails with exception', async () => {
    const mockMutate = vi.fn()
    const startServerMock = {
      mutate: mockMutate,
      isPending: false,
    }

    mockUseServerStatus.mockReturnValue({
      data: { running: false, configured: true, uptimeSeconds: 0 },
    })
    mockUseStartServer.mockReturnValue(startServerMock)
    mockUseStopServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseRestartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })

    renderWithQuery(<PreviewToolbar onReload={vi.fn()} />)

    // Simulate the onError callback being called
    const onErrorCallback = mockUseStartServer.mock.calls[0][0].onError
    const error = new Error('Network error')

    onErrorCallback(error)

    // Verify error toast was called
    expect(toast.error).toHaveBeenCalledWith('Failed to start preview environment server', {
      description: 'Network error',
    })
  })

  it('should apply correct status colors for failed state', () => {
    const serverStatusWithError: ServerStatus = {
      running: false,
      configured: true,
      uptimeSeconds: 0,
      error: 'Server failed to start',
    }

    mockUseServerStatus.mockReturnValue({ data: serverStatusWithError })
    mockUseStartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseStopServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseRestartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })

    renderWithQuery(<PreviewToolbar onReload={vi.fn()} />)

    // The server button should have error color class
    const buttons = screen.getAllByRole('button')
    const serverButton = buttons[1] // The server button is the middle button
    expect(serverButton).toHaveClass('text-status-error')
  })

  it('should not display error section when server has no error', () => {
    const serverStatusWithoutError: ServerStatus = {
      running: false,
      configured: true,
      uptimeSeconds: 0,
    }

    mockUseServerStatus.mockReturnValue({ data: serverStatusWithoutError })
    mockUseStartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseStopServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseRestartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })

    renderWithQuery(<PreviewToolbar onReload={vi.fn()} />)

    // Check that "Stopped" status is displayed (not "Failed")
    expect(screen.getByText('Stopped')).toBeInTheDocument()
    expect(screen.queryByText('Error:')).not.toBeInTheDocument()
    expect(screen.queryByText('Logs:')).not.toBeInTheDocument()
  })

  it('should display logs section only when logs are available', () => {
    const serverStatusWithErrorButNoLogs: ServerStatus = {
      running: false,
      configured: true,
      uptimeSeconds: 0,
      error: 'Server failed to start',
      logs: '', // Empty logs
    }

    mockUseServerStatus.mockReturnValue({ data: serverStatusWithErrorButNoLogs })
    mockUseStartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseStopServer.mockReturnValue({ mutate: vi.fn(), isPending: false })
    mockUseRestartServer.mockReturnValue({ mutate: vi.fn(), isPending: false })

    renderWithQuery(<PreviewToolbar onReload={vi.fn()} />)

    // Error should be displayed
    expect(screen.getByText('Error:')).toBeInTheDocument()
    expect(screen.getByText('Server failed to start')).toBeInTheDocument()

    // Logs section should not be displayed when logs are empty
    expect(screen.queryByText('Logs:')).not.toBeInTheDocument()
  })
})
