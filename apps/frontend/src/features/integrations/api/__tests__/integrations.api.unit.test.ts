import {
  type AdapterConnectionResponse,
  AdapterKind,
  type CreateAdapterConnectionRequest,
  type CreateTriggerRequest,
  RoutingMode,
  Scope,
  TriggerKind,
  type TriggerResponse,
} from '@moatless/api-client'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { integrationsApi } from '../integrations.api'

const mockListAdapters = vi.hoisted(() => vi.fn())
const mockListAdapterConnections = vi.hoisted(() => vi.fn())
const mockCreateAdapterConnection = vi.hoisted(() => vi.fn())
const mockListTriggers = vi.hoisted(() => vi.fn())
const mockCreateTrigger = vi.hoisted(() => vi.fn())
const mockUpdateTrigger = vi.hoisted(() => vi.fn())
const mockDeleteTrigger = vi.hoisted(() => vi.fn())
const mockGetTrigger = vi.hoisted(() => vi.fn())
const mockListTriggerRecentTasks = vi.hoisted(() => vi.fn())

vi.mock('@moatless/api-client/sandbox/adapters/adapters', () => ({
  listAdapters: mockListAdapters,
  listAdapterConnections: mockListAdapterConnections,
  createAdapterConnection: mockCreateAdapterConnection,
}))

vi.mock('@moatless/api-client/sandbox/triggers/triggers', () => ({
  listTriggers: mockListTriggers,
  createTrigger: mockCreateTrigger,
  updateTrigger: mockUpdateTrigger,
  deleteTrigger: mockDeleteTrigger,
  getTrigger: mockGetTrigger,
  listTriggerRecentTasks: mockListTriggerRecentTasks,
}))

describe('integrationsApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('uses generated adapter and connection handlers', async () => {
    const connectionRequest: CreateAdapterConnectionRequest = {
      adapterKind: AdapterKind.slack,
      externalAccountId: 'T123',
      sourceName: 'Workspace',
      connectionKind: 'bot_api',
      webhookSecret: null,
      transportConfig: null,
    }
    const connectionResponse: AdapterConnectionResponse = {
      id: 'connection-1',
      adapterKind: AdapterKind.slack,
      externalAccountId: 'T123',
      connectionKind: 'bot_api',
      createdBy: 'user-1',
      createdAt: '2026-05-07T10:00:00Z',
      updatedAt: '2026-05-07T10:00:00Z',
      scope: Scope.user,
    }

    mockListAdapters.mockResolvedValue(['slack'])
    mockListAdapterConnections.mockResolvedValue([connectionResponse])
    mockCreateAdapterConnection.mockResolvedValue(connectionResponse)

    await expect(integrationsApi.listAdapters()).resolves.toEqual(['slack'])
    await expect(integrationsApi.listAdapterConnections()).resolves.toEqual([connectionResponse])
    await expect(integrationsApi.createAdapterConnection(connectionRequest)).resolves.toEqual(
      connectionResponse,
    )

    expect(mockCreateAdapterConnection).toHaveBeenCalledWith(connectionRequest)
  })

  it('uses generated unified trigger handlers', async () => {
    const triggerRequest: CreateTriggerRequest = {
      kind: TriggerKind.adapter_event,
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
    const triggerResponse: TriggerResponse = {
      id: 'trigger-1',
      kind: TriggerKind.adapter_event,
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

    mockListTriggers.mockResolvedValue([triggerResponse])
    mockCreateTrigger.mockResolvedValue(triggerResponse)
    mockUpdateTrigger.mockResolvedValue(triggerResponse)
    mockGetTrigger.mockResolvedValue(triggerResponse)
    mockDeleteTrigger.mockResolvedValue(undefined)

    await expect(integrationsApi.listTriggers()).resolves.toEqual([triggerResponse])
    await expect(integrationsApi.createTrigger(triggerRequest)).resolves.toEqual(triggerResponse)
    await expect(integrationsApi.updateTrigger('trigger-1', { active: false })).resolves.toEqual(
      triggerResponse,
    )
    await expect(integrationsApi.getTrigger('trigger-1')).resolves.toEqual(triggerResponse)
    await expect(integrationsApi.deleteTrigger('trigger-1')).resolves.toBeUndefined()

    expect(mockCreateTrigger).toHaveBeenCalledWith(triggerRequest)
    expect(mockUpdateTrigger).toHaveBeenCalledWith('trigger-1', { active: false })
  })
})
