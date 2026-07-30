import type { ListTasksResponse, TaskResponse } from '@moatless/api-client'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { SidebarProvider } from '@/lib/components/ui/sidebar'
import { render, screen } from '@/test-utils/render'
import { SidebarTasks } from '../sidebar-tasks'

const BASE_TIME = '2026-01-01T00:00:00.000Z'

const closeTask = vi.fn()
let listedTasks: TaskResponse[] = []

vi.mock('@tanstack/react-router', () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/tasks/task-1">{children}</a>,
  useNavigate: () => () => Promise.resolve(),
  useParams: () => ({}),
}))

vi.mock('@moatless/api-client/sandbox/auth/auth', () => ({
  meHandler: () =>
    Promise.resolve({
      user: { id: 'user-1', login: 'user', provider: 'github', role: 'user' },
    }),
}))

vi.mock('@moatless/api-client/sandbox/sandbox/sandbox', () => ({
  liveStatusHandler: () =>
    Promise.resolve({ agentStatus: null, sandboxError: null, sandboxStatus: 'stopped' }),
}))

vi.mock('@moatless/api-client/sandbox/tasks/tasks', () => ({
  listTasks: (): Promise<ListTasksResponse> =>
    Promise.resolve({
      items: listedTasks,
      pagination: { hasMore: false, limit: 100, offset: 0, total: listedTasks.length },
    }),
  closeTask: (taskId: string, request: unknown) => {
    closeTask(taskId, request)
    return Promise.resolve(listedTasks[0])
  },
}))

vi.mock('@/features/events/contexts/global-event-stream-provider', () => ({
  useGlobalEventStream: () => ({
    subscribeToAll: () => () => {
      /* no events in this test */
    },
  }),
}))

function task(taskId: string, parentTaskId?: string): TaskResponse {
  return {
    agentType: 'claude-code',
    bindings: [],
    createdAt: BASE_TIME,
    inactiveTtlDays: 7,
    lastMessageAt: BASE_TIME,
    lastReadAt: BASE_TIME,
    name: taskId,
    ...(parentTaskId !== undefined && { parentTaskId }),
    relations: [],
    repositoryId: 'repo-1',
    repositoryName: 'Repo',
    workspaceName: 'Workspace',
    sandboxDesiredState: 'stopped',
    sandboxDesiredStateUpdatedAt: BASE_TIME,
    scope: 'user',
    status: 'open',
    tags: [],
    taskId,
    title: taskId,
    totalCostUsd: 0,
    ttlMinutes: 60,
    visibility: 'private',
  }
}

function renderSidebar() {
  return render(
    <SidebarProvider>
      <SidebarTasks />
    </SidebarProvider>,
  )
}

describe('SidebarTasks close button', () => {
  beforeEach(() => {
    closeTask.mockClear()
    listedTasks = [task('task-1'), task('task-2', 'task-1')]
  })

  it('asks how to handle children before closing a parent row', async () => {
    const user = userEvent.setup()
    renderSidebar()

    expect(await screen.findByText(/Parent · 1 child/)).toBeInTheDocument()

    await user.click(screen.getAllByRole('button', { name: 'Close task' })[0] as HTMLElement)

    expect(await screen.findByText('Close parent task?')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Close children too' }))

    expect(closeTask).toHaveBeenCalledWith('task-1', { closeChildren: true })
  })

  it('closes a childless row without asking', async () => {
    const user = userEvent.setup()
    listedTasks = [task('task-1')]
    renderSidebar()

    await screen.findByText('task-1')
    await user.click(screen.getByRole('button', { name: 'Close task' }))

    expect(screen.queryByText('Close parent task?')).not.toBeInTheDocument()
    expect(closeTask).toHaveBeenCalledWith('task-1', {})
  })
})
