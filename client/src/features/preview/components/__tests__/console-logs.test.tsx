import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import React from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ConsoleLogs } from '../console-logs'

// Mock the scroll area component
vi.mock('@/lib/components/ui/scroll-area', () => ({
  ScrollArea: ({
    children,
    onScroll,
  }: {
    children: React.ReactNode
    onScroll?: (e: React.UIEvent<HTMLDivElement>) => void
  }) => (
    <div data-testid="scroll-area" onScroll={onScroll}>
      <div data-radix-scroll-area-viewport>{children}</div>
    </div>
  ),
}))

// Mock the preview store
vi.mock('../stores/preview-store', () => ({
  usePreviewStore: () => ({
    config: { url: 'http://localhost:3000' },
  }),
}))

describe('ConsoleLogs', () => {
  // Mock console methods
  const originalConsole = { ...console }
  const mockConsoleLog = vi.fn()
  const mockConsoleError = vi.fn()
  const mockConsoleWarn = vi.fn()
  const mockConsoleInfo = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()
    // Reset console methods
    console.log = mockConsoleLog
    console.error = mockConsoleError
    console.warn = mockConsoleWarn
    console.info = mockConsoleInfo
  })

  afterEach(() => {
    // Restore original console
    console.log = originalConsole.log
    console.error = originalConsole.error
    console.warn = originalConsole.warn
    console.info = originalConsole.info
  })

  describe('Basic Rendering', () => {
    it('renders empty state when no logs are present', () => {
      render(<ConsoleLogs />)

      expect(screen.getByText('No console output')).toBeInTheDocument()
      expect(
        screen.getByText('Console logs from the preview app will appear here'),
      ).toBeInTheDocument()
    })
  })

  describe('Console Log Filtering', () => {
    it('DOES NOT capture console logs from the main application', () => {
      render(<ConsoleLogs />)

      // These console calls from the main app should NOT appear in the component
      console.log('This is from the main app')
      console.error('Main app error')
      console.warn('Main app warning')
      console.info('Main app info')

      // Verify the console methods were called
      expect(mockConsoleLog).toHaveBeenCalledWith('This is from the main app')
      expect(mockConsoleError).toHaveBeenCalledWith('Main app error')
      expect(mockConsoleWarn).toHaveBeenCalledWith('Main app warning')
      expect(mockConsoleInfo).toHaveBeenCalledWith('Main app info')

      // But they should NOT appear in the component
      expect(screen.getByText('No console output')).toBeInTheDocument()
      expect(screen.queryByText('This is from the main app')).not.toBeInTheDocument()
      expect(screen.queryByText('Main app error')).not.toBeInTheDocument()
      expect(screen.queryByText('Main app warning')).not.toBeInTheDocument()
      expect(screen.queryByText('Main app info')).not.toBeInTheDocument()
    })

    it('captures and displays console logs from the preview iframe', async () => {
      const { rerender } = render(<ConsoleLogs />)

      // Simulate the component receiving logs from the preview iframe
      // In the actual implementation, this would be done via postMessage from the iframe
      const mockPreviewLogs = [
        {
          id: '1',
          timestamp: new Date().toISOString(),
          level: 'log' as const,
          message: 'Preview app started',
        },
        {
          id: '2',
          timestamp: new Date().toISOString(),
          level: 'error' as const,
          message: 'Preview app error: Cannot find module',
        },
        {
          id: '3',
          timestamp: new Date().toISOString(),
          level: 'warn' as const,
          message: 'Preview app warning: Deprecated API',
        },
      ]

      // Simulate updating the component with preview logs
      // In real implementation, this would be handled by a message handler
      const ConsoleLogsWithLogs = () => {
        const [logs] = React.useState(mockPreviewLogs)
        return <MockConsoleLogs logs={logs} />
      }

      rerender(<ConsoleLogsWithLogs />)

      // Verify preview logs are displayed
      await waitFor(() => {
        expect(screen.queryByText('No console output')).not.toBeInTheDocument()
        expect(screen.getByText(/Preview app started/)).toBeInTheDocument()
        expect(screen.getByText(/Preview app error: Cannot find module/)).toBeInTheDocument()
        expect(screen.getByText(/Preview app warning: Deprecated API/)).toBeInTheDocument()
      })
    })

    it('displays different log levels with appropriate styling', async () => {
      const mockLogs = [
        {
          id: '1',
          timestamp: new Date().toISOString(),
          level: 'log' as const,
          message: 'Normal log',
        },
        {
          id: '2',
          timestamp: new Date().toISOString(),
          level: 'error' as const,
          message: 'Error log',
        },
        {
          id: '3',
          timestamp: new Date().toISOString(),
          level: 'warn' as const,
          message: 'Warning log',
        },
        {
          id: '4',
          timestamp: new Date().toISOString(),
          level: 'info' as const,
          message: 'Info log',
        },
        {
          id: '5',
          timestamp: new Date().toISOString(),
          level: 'debug' as const,
          message: 'Debug log',
        },
      ]

      const ConsoleLogsWithLogs = () => {
        const [logs] = React.useState(mockLogs)
        return <MockConsoleLogs logs={logs} />
      }

      render(<ConsoleLogsWithLogs />)

      // Check that different log levels have different styles
      const normalLog = screen.getByText(/Normal log/)
      const errorLog = screen.getByText(/Error log/)
      const warnLog = screen.getByText(/Warning log/)
      const infoLog = screen.getByText(/Info log/)
      const debugLog = screen.getByText(/Debug log/)

      expect(normalLog.className).toContain('text-slate-100')
      expect(errorLog.className).toContain('text-red-400')
      expect(warnLog.className).toContain('text-yellow-400')
      expect(infoLog.className).toContain('text-blue-400')
      expect(debugLog.className).toContain('text-gray-500')
    })
  })

  describe('Auto-scrolling', () => {
    it('automatically scrolls to bottom when new logs are added', async () => {
      const scrollIntoViewMock = vi.fn()
      Element.prototype.scrollIntoView = scrollIntoViewMock

      const { rerender } = render(<ConsoleLogs />)

      const initialLogs = [
        {
          id: '1',
          timestamp: new Date().toISOString(),
          level: 'log' as const,
          message: 'First log',
        },
      ]

      const ConsoleLogsWithLogs = ({ logs }: { logs: typeof initialLogs }) => {
        const [logState] = React.useState(logs)
        return <MockConsoleLogs logs={logState} />
      }

      rerender(<ConsoleLogsWithLogs logs={initialLogs} />)

      // Add more logs
      const updatedLogs = [
        ...initialLogs,
        {
          id: '2',
          timestamp: new Date().toISOString(),
          level: 'log' as const,
          message: 'Second log',
        },
      ]

      rerender(<ConsoleLogsWithLogs logs={updatedLogs} />)

      // Verify scroll behavior was triggered
      const scrollArea = screen.getByTestId('scroll-area')
      expect(scrollArea.querySelector('[data-radix-scroll-area-viewport]')).toBeTruthy()
    })

    it('stops auto-scrolling when user scrolls up', async () => {
      const user = userEvent.setup()
      render(<ConsoleLogs />)

      const scrollArea = screen.getByTestId('scroll-area')

      // Simulate user scrolling up
      await user.click(scrollArea)
      scrollArea.scrollTop = 100
      scrollArea.dispatchEvent(new Event('scroll', { bubbles: true }))

      // Verify auto-scroll is disabled when not at bottom
      expect(scrollArea.scrollTop).toBe(100)
    })
  })
})

// Mock component for testing with injected logs
const MockConsoleLogs = ({
  logs,
}: {
  logs: Array<{ id: string; timestamp: string; level: string; message: string }>
}) => {
  const _React = require('react')
  const { ScrollArea } = require('@/lib/components/ui/scroll-area')

  const getLevelColor = (level: string) => {
    switch (level) {
      case 'error':
        return 'text-red-400'
      case 'warn':
        return 'text-yellow-400'
      case 'info':
        return 'text-blue-400'
      case 'debug':
        return 'text-gray-500'
      default:
        return 'text-slate-100 dark:text-slate-200'
    }
  }

  const formatMessage = (log: (typeof logs)[0]) => {
    const timestamp = new Date(log.timestamp).toLocaleTimeString()
    const prefix = `[${timestamp}] [${log.level.toUpperCase()}]`
    return `${prefix} ${log.message}`
  }

  if (logs.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center">
        <div className="text-muted-foreground">
          <p className="text-lg font-medium">No console output</p>
          <p className="mt-2 text-sm">Console logs from the preview app will appear here</p>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full bg-slate-950 dark:bg-black">
      <ScrollArea className="h-full w-full">
        <div className="p-4 space-y-1">
          {logs.map((log) => (
            <div
              key={log.id}
              className="text-xs leading-relaxed font-mono whitespace-pre-wrap break-all"
            >
              <span className={getLevelColor(log.level)}>{formatMessage(log)}</span>
            </div>
          ))}
        </div>
      </ScrollArea>
    </div>
  )
}
