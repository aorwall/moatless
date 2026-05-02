import type { ScheduledJobInfo } from '@moatless/api-client'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ScheduledJobsPill } from '../scheduled-jobs-pill'

describe('ScheduledJobsPill', () => {
  it('renders nothing when there are no jobs', () => {
    const { container } = render(<ScheduledJobsPill jobs={[]} />)
    expect(container.firstChild).toBeNull()
  })

  it('renders pill with count and lists wakeup rows in popover', () => {
    const jobs: ScheduledJobInfo[] = [
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
    expect(trigger).toHaveTextContent('1')

    fireEvent.click(trigger)

    expect(screen.getByText('Resume work')).toBeInTheDocument()
    expect(screen.getByText(/^in /)).toBeInTheDocument()
  })
})
