import type { ScheduledJobInfo } from '@moatless/api-client'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ScheduledJobsPill } from '../scheduled-jobs-pill'

describe('ScheduledJobsPill', () => {
  it('renders nothing when there are no jobs', () => {
    const { container } = render(<ScheduledJobsPill jobs={[]} />)
    expect(container.firstChild).toBeNull()
  })

  it('renders pill with count and lists monitor + wakeup rows in popover', () => {
    const jobs: ScheduledJobInfo[] = [
      {
        id: 'm1',
        registeredAtUnixMs: 1_700_000_000_000,
        kind: 'monitor',
        description: 'Tail logs',
        commandPreview: 'tail -f /var/log/app.log',
        timeoutMs: 30_000,
        persistent: true,
      },
      {
        id: 'w1',
        registeredAtUnixMs: 1_700_000_000_000,
        kind: 'wakeup',
        reason: 'Resume work',
        promptPreview: 'continue the loop',
        wakeupAtUnixMs: Date.now() + 60_000,
      },
    ]

    render(<ScheduledJobsPill jobs={jobs} />)

    const trigger = screen.getByTitle('Scheduled jobs')
    expect(trigger).toBeInTheDocument()
    expect(trigger).toHaveTextContent('2')

    fireEvent.click(trigger)

    expect(screen.getByText('Tail logs')).toBeInTheDocument()
    expect(screen.getByText('tail -f /var/log/app.log')).toBeInTheDocument()
    expect(screen.getByText('persist')).toBeInTheDocument()
    expect(screen.getByText('Resume work')).toBeInTheDocument()
    expect(screen.getByText(/^in /)).toBeInTheDocument()
  })
})
