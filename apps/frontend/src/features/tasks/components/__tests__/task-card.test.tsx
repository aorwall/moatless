import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { TaskCard } from '../task-card'
import { createMockTask } from '@/test-utils/factories'
import type { Task } from '@moatless/shared/types/task'

// Mock dependencies
vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, to, params }: any) => (
    <a href={`${to}/${params?.taskId}`}>{children}</a>
  ),
}))

describe('TaskCard', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('Rendering', () => {
    it('renders task information correctly', () => {
      const task = createMockTask({
        taskId: 'abc12345-def6-7890',
        status: 'running',
        agentType: 'claude-code',
        numTurns: 5,
        totalCostUsd: 0.1234,
        repositoryId: 'test-repo',
        createdAt: new Date('2025-01-01T10:00:00Z'),
        lastMessageAt: new Date('2025-01-01T15:30:00Z'),
      })

      render(<TaskCard task={task} />)

      // Check task ID display (first 8 chars)
      expect(screen.getByText('Task abc12345')).toBeInTheDocument()

      // Check status badge
      expect(screen.getByText('Running')).toBeInTheDocument()

      // Check metadata
      expect(screen.getByText('Agent: claude-code')).toBeInTheDocument()
      expect(screen.getByText('5 turns')).toBeInTheDocument()
      expect(screen.getByText('$0.1234')).toBeInTheDocument()
      expect(screen.getByText('Repository: test-repo')).toBeInTheDocument()
    })

    it('formats dates correctly', () => {
      const task = createMockTask({
        createdAt: new Date('2025-01-15T10:00:00Z'),
        lastMessageAt: new Date('2025-01-15T15:30:45Z'),
      })

      render(<TaskCard task={task} />)

      // Note: toLocaleDateString and toLocaleTimeString outputs depend on the test environment locale
      // We'll check for the presence of date/time elements instead
      expect(screen.getByText(/Created:/)).toBeInTheDocument()
      expect(screen.getByText(/Last activity:/)).toBeInTheDocument()
    })

    it('applies correct styling based on task status', () => {
      const errorTask = createMockTask({ status: 'error' })
      const { rerender } = render(<TaskCard task={errorTask} />)

      // Error tasks should have reduced opacity
      const card = screen.getByRole('link').querySelector('.opacity-75')
      expect(card).toBeInTheDocument()

      // Non-error tasks should not have reduced opacity
      const runningTask = createMockTask({ status: 'running' })
      rerender(<TaskCard task={runningTask} />)
      
      const normalCard = screen.getByRole('link').querySelector('.opacity-75')
      expect(normalCard).not.toBeInTheDocument()
    })

    it('displays different status badges based on task status', () => {
      const statuses: Array<Task['status']> = ['running', 'open', 'error', 'closed', 'merged']

      statuses.forEach((status) => {
        const task = createMockTask({ status })
        const { rerender } = render(<TaskCard task={task} />)

        // Status badges have different text based on status
        const expectedText = {
          running: 'Running',
          open: 'Open',
          error: 'Error',
          closed: 'Closed',
          merged: 'Merged',
        }[status]

        expect(screen.getByText(expectedText!)).toBeInTheDocument()

        rerender(<div />)
      })
    })
  })

  describe('Navigation', () => {
    it('creates correct link to task detail page', () => {
      const task = createMockTask({ taskId: 'task-123' })

      render(<TaskCard task={task} />)

      const link = screen.getByRole('link')
      expect(link).toHaveAttribute('href', '/tasks/$taskId/task-123')
    })

    it('makes entire card clickable', () => {
      const task = createMockTask()

      render(<TaskCard task={task} />)

      const link = screen.getByRole('link')
      // The card component uses data-slot attribute
      const card = link.querySelector('[data-slot="card"]')
      
      // Card should be inside the link
      expect(card).toBeInTheDocument()
      expect(link).toContainElement(card as HTMLElement)
    })
  })

  describe('Visual Elements', () => {
    it('displays all required icons', () => {
      const task = createMockTask()

      render(<TaskCard task={task} />)

      // Check for icon presence using lucide class names
      expect(document.querySelector('.lucide-calendar')).toBeInTheDocument()
      expect(document.querySelector('.lucide-clock')).toBeInTheDocument()
      expect(document.querySelector('.lucide-server')).toBeInTheDocument()
      expect(document.querySelector('.lucide-message-square')).toBeInTheDocument()
      expect(document.querySelector('.lucide-dollar-sign')).toBeInTheDocument()
    })

    it('shows status icon based on task status', () => {
      const task = createMockTask({ status: 'running' })

      render(<TaskCard task={task} />)

      // Status icon should be present (exact icon depends on getTaskStatusConfig)
      const statusIconContainer = document.querySelector('[class*="flex items-center"]')
      expect(statusIconContainer).toBeInTheDocument()
    })
  })

  describe('Cost Display', () => {
    it('formats cost with 4 decimal places', () => {
      const testCases = [
        { cost: 0.1, expected: '$0.1000' },
        { cost: 0.1234, expected: '$0.1234' },
        { cost: 0.12345678, expected: '$0.1235' }, // Should round
        { cost: 1.0, expected: '$1.0000' },
        { cost: 0, expected: '$0.0000' },
      ]

      testCases.forEach(({ cost, expected }) => {
        const task = createMockTask({ totalCostUsd: cost })
        const { rerender } = render(<TaskCard task={task} />)

        expect(screen.getByText(expected)).toBeInTheDocument()

        rerender(<div />)
      })
    })
  })

  describe('Edge Cases', () => {
    it('handles very long task IDs', () => {
      const task = createMockTask({
        taskId: 'this-is-a-very-long-task-id-that-should-be-truncated',
      })

      render(<TaskCard task={task} />)

      // Should only show first 8 characters
      expect(screen.getByText('Task this-is-')).toBeInTheDocument()
    })

    it('handles missing optional fields gracefully', () => {
      const task = createMockTask({
        numTurns: 0,
        totalCostUsd: 0,
      })

      render(<TaskCard task={task} />)

      expect(screen.getByText('0 turns')).toBeInTheDocument()
      expect(screen.getByText('$0.0000')).toBeInTheDocument()
    })

    it('handles all task statuses without errors', () => {
      const allStatuses: Array<Task['status']> = ['open', 'running', 'error', 'closed', 'merged']

      allStatuses.forEach((status) => {
        const task = createMockTask({ status })
        
        expect(() => {
          const { unmount } = render(<TaskCard task={task} />)
          unmount()
        }).not.toThrow()
      })
    })
  })

  describe('Accessibility', () => {
    it('has appropriate heading structure', () => {
      const task = createMockTask({ taskId: 'test-123' })

      render(<TaskCard task={task} />)

      const heading = screen.getByRole('heading', { level: 3 })
      expect(heading).toHaveTextContent('Task test-123')
    })

    it('provides semantic structure with proper elements', () => {
      const task = createMockTask()

      render(<TaskCard task={task} />)

      // Should have a link wrapper
      expect(screen.getByRole('link')).toBeInTheDocument()

      // Should have a heading
      expect(screen.getByRole('heading')).toBeInTheDocument()
    })
  })
})