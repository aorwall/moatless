import type { ManagedCommand, ScheduledJobInfo } from '@moatless/api-client'
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ManagedCommandsSection } from '../managed-commands-section'

function command(overrides: Partial<ManagedCommand>): ManagedCommand {
  return {
    id: 'cmd-1',
    command: 'cargo test --no-run',
    label: 'cargo test --no-run',
    pid: 1234,
    startedAtUnixMs: 1,
    timeoutSecs: 3600,
    deadlineUnixMs: 2,
    state: 'running',
    exitCode: null,
    logPath: '/tmp/moat-commands/t/cmd-1.log',
    finishedAtUnixMs: 0,
    ...overrides,
  }
}

describe('ManagedCommandsSection', () => {
  it('renders nothing when there are no commands and no watch', () => {
    const { container } = render(<ManagedCommandsSection commands={[]} scheduledJobs={[]} />)
    expect(container.firstChild).toBeNull()
  })

  it('lists commands with their outcome', () => {
    const commands: ManagedCommand[] = [
      command({}),
      command({ id: 'cmd-2', label: 'mvn verify', state: 'exited', exitCode: 1 }),
      command({ id: 'cmd-3', label: 'sleep 999', state: 'timedOut', timeoutSecs: 60 }),
    ]

    render(<ManagedCommandsSection commands={commands} scheduledJobs={[]} />)

    expect(screen.getByText('Commands')).toBeInTheDocument()
    expect(screen.getByText('pid 1234')).toBeInTheDocument()
    expect(screen.getByText('cargo test --no-run')).toBeInTheDocument()
    expect(screen.getByText('exit 1')).toBeInTheDocument()
    expect(screen.getByText('timeout 60s')).toBeInTheDocument()
  })

  it('notes automatic resume when a watch is armed but nothing is listed yet', () => {
    const scheduledJobs: ScheduledJobInfo[] = [
      {
        id: 'bg1',
        registeredAtUnixMs: 1,
        kind: 'backgroundWatch',
        commandCount: 1,
        commands: ['cargo build'],
      },
    ]

    render(<ManagedCommandsSection commands={[]} scheduledJobs={scheduledJobs} />)

    expect(screen.getByText(/resume automatically when 1 command finishes/i)).toBeInTheDocument()
  })
})
