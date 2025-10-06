import { render, screen, waitFor } from '@testing-library/react'
import { userEvent } from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useServerStatus, useStartServer } from '@/features/preview/hooks/use-server'
import { useServerEvents } from '@/features/preview/hooks/use-server-events'
import { createMockLoadingQuery, createMockQuery } from '@/test-utils/mocks'
import { Preview } from '../preview'

vi.mock('@/features/preview/hooks/use-server', () => ({
  useServerStatus: vi.fn(),
  useStartServer: vi.fn(),
}))

vi.mock('@/features/preview/hooks/use-server-events', () => ({
  useServerEvents: vi.fn(),
}))

vi.mock('../preview-header', () => ({
  PreviewHeader: () => <div data-testid="preview-header">Preview Header</div>,
}))

describe('Preview', () => {
  const defaultTaskId = 'task-123'
  const mockStartServer = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(useStartServer).mockReturnValue({
      mutate: mockStartServer,
      isPending: false,
    } as any)
    vi.mocked(useServerEvents).mockReturnValue({
      serverStatus: null,
      serverError: null,
      serverPort: undefined,
      serverUrl: undefined,
      lastServerEvent: null,
    })
  })

  describe('Server Status States', () => {
    it('shows stopped state when server is not running', () => {
      vi.mocked(useServerStatus).mockReturnValue(createMockQuery({ status: 'stopped' }))

      render(<Preview taskId={defaultTaskId} />)

      expect(screen.getByText('The preview server is not running')).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /start server/i })).toBeInTheDocument()
    })

    it('shows starting state when server is initializing', () => {
      vi.mocked(useServerStatus).mockReturnValue(createMockQuery({ status: 'starting' }))

      render(<Preview taskId={defaultTaskId} />)

      expect(screen.getByText('Server is starting...')).toBeInTheDocument()
      expect(
        screen.getByText(/please wait while the preview server initializes/i),
      ).toBeInTheDocument()
    })

    it('shows started state with iframe when server is running', () => {
      vi.mocked(useServerStatus).mockReturnValue(
        createMockQuery({
          status: 'started',
          port: 3000,
          url: 'http://localhost:3000',
          proxyUrl: 'http://localhost:8080',
        }),
      )

      render(<Preview taskId={defaultTaskId} />)

      const iframe = screen.getByTitle('Application Preview')
      expect(iframe).toBeInTheDocument()
      expect(iframe).toHaveAttribute('src', 'http://localhost:8080/')
    })

    it('shows failed state with error message', () => {
      vi.mocked(useServerStatus).mockReturnValue(createMockQuery({ status: 'failed' }))
      vi.mocked(useServerEvents).mockReturnValue({
        serverStatus: 'failed',
        serverError: 'Port already in use',
        serverPort: undefined,
        serverUrl: undefined,
        lastServerEvent: null,
      })

      render(<Preview taskId={defaultTaskId} />)

      expect(screen.getByText('Server failed to start')).toBeInTheDocument()
      expect(screen.getByText('Port already in use')).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument()
    })
  })

  describe('SSE Event Priority', () => {
    it('uses SSE status over query status when both available', () => {
      vi.mocked(useServerStatus).mockReturnValue(createMockQuery({ status: 'stopped' }))
      vi.mocked(useServerEvents).mockReturnValue({
        serverStatus: 'started',
        serverError: null,
        serverPort: 3000,
        serverUrl: 'http://localhost:3000',
        lastServerEvent: null,
      })

      render(<Preview taskId={defaultTaskId} />)

      expect(screen.queryByText('The preview server is not running')).not.toBeInTheDocument()
    })

    it('falls back to query status when SSE status is null', () => {
      vi.mocked(useServerStatus).mockReturnValue(createMockQuery({ status: 'starting' }))
      vi.mocked(useServerEvents).mockReturnValue({
        serverStatus: null,
        serverError: null,
        serverPort: undefined,
        serverUrl: undefined,
        lastServerEvent: null,
      })

      render(<Preview taskId={defaultTaskId} />)

      expect(screen.getByText('Server is starting...')).toBeInTheDocument()
    })

    it('uses SSE error message when server fails', () => {
      vi.mocked(useServerStatus).mockReturnValue(createMockQuery({ status: 'failed' }))
      vi.mocked(useServerEvents).mockReturnValue({
        serverStatus: 'failed',
        serverError: 'Failed to install dependencies',
        serverPort: undefined,
        serverUrl: undefined,
        lastServerEvent: null,
      })

      render(<Preview taskId={defaultTaskId} />)

      expect(screen.getByText('Failed to install dependencies')).toBeInTheDocument()
    })
  })

  describe('Start Server Button', () => {
    it('calls startServer mutation when Start Server clicked', async () => {
      vi.mocked(useServerStatus).mockReturnValue(createMockQuery({ status: 'stopped' }))

      render(<Preview taskId={defaultTaskId} />)

      const startButton = screen.getByRole('button', { name: /start server/i })
      await userEvent.click(startButton)

      expect(mockStartServer).toHaveBeenCalledWith(defaultTaskId)
    })

    it('disables button and shows loading state when starting', () => {
      vi.mocked(useServerStatus).mockReturnValue(createMockQuery({ status: 'stopped' }))
      vi.mocked(useStartServer).mockReturnValue({
        mutate: mockStartServer,
        isPending: true,
      } as any)

      render(<Preview taskId={defaultTaskId} />)

      const startButton = screen.getByRole('button', {
        name: /starting server/i,
      })
      expect(startButton).toBeDisabled()
      expect(screen.getByText('Starting Server...')).toBeInTheDocument()
    })

    it('allows retry when server fails to start', async () => {
      vi.mocked(useServerStatus).mockReturnValue(createMockQuery({ status: 'failed' }))
      vi.mocked(useServerEvents).mockReturnValue({
        serverStatus: 'failed',
        serverError: 'Failed to start',
        serverPort: undefined,
        serverUrl: undefined,
        lastServerEvent: null,
      })

      render(<Preview taskId={defaultTaskId} />)

      const retryButton = screen.getByRole('button', { name: /retry/i })
      await userEvent.click(retryButton)

      expect(mockStartServer).toHaveBeenCalledWith(defaultTaskId)
    })
  })

  describe('Loading States', () => {
    it('shows loading skeleton during query load', () => {
      vi.mocked(useServerStatus).mockReturnValue(createMockLoadingQuery())

      render(<Preview taskId={defaultTaskId} />)

      expect(screen.getByText('The preview server is not running')).toBeInTheDocument()
    })

    it('defaults to stopped when no data available', () => {
      vi.mocked(useServerStatus).mockReturnValue(createMockQuery(undefined as any))

      render(<Preview taskId={defaultTaskId} />)

      expect(screen.getByText('The preview server is not running')).toBeInTheDocument()
    })
  })

  describe('URL Handling', () => {
    it('uses proxyUrl when available', () => {
      vi.mocked(useServerStatus).mockReturnValue(
        createMockQuery({
          status: 'started',
          port: 3000,
          url: 'http://localhost:3000',
          proxyUrl: 'http://localhost:8080',
        }),
      )

      render(<Preview taskId={defaultTaskId} />)

      const iframe = screen.getByTitle('Application Preview')
      expect(iframe).toHaveAttribute('src', 'http://localhost:8080/')
    })

    it('falls back to direct url when proxyUrl not available', () => {
      vi.mocked(useServerStatus).mockReturnValue(
        createMockQuery({
          status: 'started',
          url: 'http://localhost:3000',
        }),
      )

      render(<Preview taskId={defaultTaskId} />)

      const iframe = screen.getByTitle('Application Preview')
      expect(iframe).toHaveAttribute('src', 'http://localhost:3000/')
    })

    it('shows error when server started but no URL available', () => {
      vi.mocked(useServerStatus).mockReturnValue(
        createMockQuery({
          status: 'started',
        }),
      )

      render(<Preview taskId={defaultTaskId} />)

      expect(screen.getByText('Preview URL is not available')).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /restart server/i })).toBeInTheDocument()
    })
  })

  describe('Inspector Toggle', () => {
    it('toggles inspector active state on button click', async () => {
      vi.mocked(useServerStatus).mockReturnValue(
        createMockQuery({
          status: 'started',
          port: 3000,
          url: 'http://localhost:3000',
          proxyUrl: 'http://localhost:8080',
        }),
      )

      render(<Preview taskId={defaultTaskId} />)

      await waitFor(() => {
        expect(screen.getByTitle('Application Preview')).toBeInTheDocument()
      })
    })
  })
})
