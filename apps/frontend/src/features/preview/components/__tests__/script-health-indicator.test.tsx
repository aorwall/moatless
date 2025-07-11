import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { TooltipProvider } from '@/lib/components/ui/tooltip'
import type { ScriptHealthStatus } from '../../types'
import { ScriptHealthIndicator } from '../script-health-indicator'

describe('ScriptHealthIndicator', () => {
  const renderWithTooltip = (status: ScriptHealthStatus, className?: string) => {
    return render(
      <TooltipProvider>
        <ScriptHealthIndicator status={status} className={className} />
      </TooltipProvider>,
    )
  }

  describe('Visual Indicators', () => {
    it('renders with not-injected status', () => {
      renderWithTooltip('not-injected')

      expect(screen.getByText('Inspector')).toBeInTheDocument()
      const icon = document.querySelector('.text-muted-foreground')
      expect(icon).toBeInTheDocument()
      // Check for XCircle icon
      expect(document.querySelector('svg')).toBeInTheDocument()
    })

    it('renders with injected-not-reachable status', () => {
      renderWithTooltip('injected-not-reachable')

      expect(screen.getByText('Inspector')).toBeInTheDocument()
      const icon = document.querySelector('.text-yellow-500')
      expect(icon).toBeInTheDocument()
      // Check for AlertCircle icon
      expect(document.querySelector('svg')).toBeInTheDocument()
    })

    it('renders with injected-reachable status', () => {
      renderWithTooltip('injected-reachable')

      expect(screen.getByText('Inspector')).toBeInTheDocument()
      const icon = document.querySelector('.text-green-500')
      expect(icon).toBeInTheDocument()
      // Check for CheckCircle2 icon
      expect(document.querySelector('svg')).toBeInTheDocument()
    })
  })

  describe('Tooltip Content', () => {
    it('shows correct tooltip for not-injected status', async () => {
      const user = userEvent.setup()
      renderWithTooltip('not-injected')

      const trigger = screen.getByText('Inspector').parentElement
      await user.hover(trigger!)

      // Wait for tooltip to appear - use getAllByText to handle duplicates from Radix tooltip implementation
      const tooltipTitles = await screen.findAllByText('Script Not Injected')
      expect(tooltipTitles.length).toBeGreaterThan(0)

      const tooltipDescriptions = screen.getAllByText(
        'Inspector script has not been injected into the preview',
      )
      expect(tooltipDescriptions.length).toBeGreaterThan(0)
    })

    it('shows correct tooltip for injected-not-reachable status', async () => {
      const user = userEvent.setup()
      renderWithTooltip('injected-not-reachable')

      const trigger = screen.getByText('Inspector').parentElement
      await user.hover(trigger!)

      const tooltipTitles = await screen.findAllByText('Script Injected (Not Reachable)')
      expect(tooltipTitles.length).toBeGreaterThan(0)

      const tooltipDescriptions = screen.getAllByText(
        'Inspector script was injected but is not responding to messages',
      )
      expect(tooltipDescriptions.length).toBeGreaterThan(0)
    })

    it('shows correct tooltip for injected-reachable status', async () => {
      const user = userEvent.setup()
      renderWithTooltip('injected-reachable')

      const trigger = screen.getByText('Inspector').parentElement
      await user.hover(trigger!)

      const tooltipTitles = await screen.findAllByText('Script Active')
      expect(tooltipTitles.length).toBeGreaterThan(0)

      const tooltipDescriptions = screen.getAllByText(
        'Inspector script is injected and actively communicating',
      )
      expect(tooltipDescriptions.length).toBeGreaterThan(0)
    })
  })

  describe('Custom Styling', () => {
    it('applies custom className', () => {
      const { container } = renderWithTooltip('injected-reachable', 'custom-class')

      const indicator = container.querySelector('.custom-class')
      expect(indicator).toBeInTheDocument()
    })
  })
})
