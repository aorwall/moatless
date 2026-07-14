import type { BackgroundProcess, ScheduledJobInfo } from '@moatless/api-client'
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { BackgroundProcessesSection } from '../background-processes-section'

describe('BackgroundProcessesSection', () => {
  it('renders nothing when there are no processes and no watch', () => {
    const { container } = render(<BackgroundProcessesSection processes={[]} scheduledJobs={[]} />)
    expect(container.firstChild).toBeNull()
  })

  it('lists live background processes with pid and command', () => {
    const processes: BackgroundProcess[] = [
      { pid: 1234, command: 'cargo test --no-run' },
      { pid: 1235, command: 'sleep 30' },
    ]

    render(<BackgroundProcessesSection processes={processes} scheduledJobs={[]} />)

    expect(screen.getByText('Background processes')).toBeInTheDocument()
    expect(screen.getByText('1234')).toBeInTheDocument()
    expect(screen.getByText('cargo test --no-run')).toBeInTheDocument()
    expect(screen.getByText('sleep 30')).toBeInTheDocument()
  })

  it('notes automatic resume when a background watch is armed but nothing is listed yet', () => {
    const scheduledJobs: ScheduledJobInfo[] = [
      {
        id: 'bg1',
        registeredAtUnixMs: 1,
        kind: 'backgroundWatch',
        processCount: 1,
        commands: ['cargo build'],
      },
    ]

    render(<BackgroundProcessesSection processes={[]} scheduledJobs={scheduledJobs} />)

    expect(
      screen.getByText(/resume automatically when 1 background job finish/i),
    ).toBeInTheDocument()
  })
})
