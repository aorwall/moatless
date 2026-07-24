import {
  AdapterKind,
  type CreateLoopRequest,
  type Loop,
  LoopKind,
  RoutingMode,
  Scope,
} from '@moatless/api-client'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { loopsApi } from '../loops.api'

const mockListLoops = vi.hoisted(() => vi.fn())
const mockCreateLoop = vi.hoisted(() => vi.fn())
const mockUpdateLoop = vi.hoisted(() => vi.fn())
const mockDeleteLoop = vi.hoisted(() => vi.fn())
const mockGetLoop = vi.hoisted(() => vi.fn())
const mockListLoopRecentTasks = vi.hoisted(() => vi.fn())

vi.mock('@moatless/api-client/sandbox/loops/loops', () => ({
  listLoops: mockListLoops,
  createLoop: mockCreateLoop,
  updateLoop: mockUpdateLoop,
  deleteLoop: mockDeleteLoop,
  getLoop: mockGetLoop,
  listLoopRecentTasks: mockListLoopRecentTasks,
}))

describe('loopsApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('uses generated unified loop handlers', async () => {
    const loopRequest: CreateLoopRequest = {
      kind: LoopKind.adapter_event,
      active: true,
      rule: {
        repositoryId: 'repo-1',
        routingMode: RoutingMode.by_subject,
        prompt: null,
        taskName: null,
        tagIds: [],
      },
      source: {
        adapterKind: AdapterKind.slack,
        connectionId: 'connection-1',
        sourceMatcher: 'C123',
        sourceName: '#triage',
        eventFilter: {},
        adapterConfig: { trigger: 'mention_only' },
      },
    }
    const loopResponse: Loop = {
      id: 'loop-1',
      kind: LoopKind.adapter_event,
      name: null,
      active: true,
      createdBy: 'user-1',
      createdAt: '2026-05-07T10:00:00Z',
      updatedAt: '2026-05-07T11:00:00Z',
      scope: Scope.user,
      rule: {
        repositoryId: 'repo-1',
        routingMode: RoutingMode.by_subject,
        ongoingTaskId: null,
        taskName: null,
        prompt: null,
        tagIds: [],
        runAsUserId: null,
        agentType: 'claude-code',
      },
      source: {
        adapterKind: AdapterKind.slack,
        connectionId: 'connection-1',
        sourceMatcher: 'C123',
        sourceName: '#triage',
        eventFilter: {},
        adapterConfig: { trigger: 'mention_only' },
        allowDirectSend: false,
        closeTaskOnClose: true,
        notifyOnSuccess: false,
      },
    }

    mockListLoops.mockResolvedValue([loopResponse])
    mockCreateLoop.mockResolvedValue(loopResponse)
    mockUpdateLoop.mockResolvedValue(loopResponse)
    mockGetLoop.mockResolvedValue(loopResponse)
    mockDeleteLoop.mockResolvedValue(undefined)

    await expect(loopsApi.listLoops()).resolves.toEqual([loopResponse])
    await expect(loopsApi.createLoop(loopRequest)).resolves.toEqual(loopResponse)
    await expect(loopsApi.updateLoop('loop-1', { active: false })).resolves.toEqual(loopResponse)
    await expect(loopsApi.getLoop('loop-1')).resolves.toEqual(loopResponse)
    await expect(loopsApi.deleteLoop('loop-1')).resolves.toBeUndefined()

    expect(mockCreateLoop).toHaveBeenCalledWith(loopRequest)
    expect(mockUpdateLoop).toHaveBeenCalledWith('loop-1', { active: false })
  })
})
