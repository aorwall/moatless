import {
  type CreateProviderConnectionRequest,
  type CreateTaskTriggerRequest,
  IntegrationProvider,
  type ProviderConnectionResponse,
  RoutingMode,
  Scope,
  type TaskTriggerResponse,
} from '@moatless/api-client'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { integrationsApi } from '../integrations.api'

const mockListIntegrationProviders = vi.hoisted(() => vi.fn())
const mockListProviderConnections = vi.hoisted(() => vi.fn())
const mockCreateProviderConnection = vi.hoisted(() => vi.fn())
const mockListTaskRoutes = vi.hoisted(() => vi.fn())
const mockCreateTaskRoute = vi.hoisted(() => vi.fn())
const mockUpdateTaskRoute = vi.hoisted(() => vi.fn())
const mockListTaskTriggers = vi.hoisted(() => vi.fn())
const mockCreateTaskTrigger = vi.hoisted(() => vi.fn())
const mockUpdateTaskTrigger = vi.hoisted(() => vi.fn())
const mockDeleteTaskTrigger = vi.hoisted(() => vi.fn())

vi.mock('@moatless/api-client/sandbox/integrations/integrations', () => ({
  listIntegrationProviders: mockListIntegrationProviders,
  listProviderConnections: mockListProviderConnections,
  createProviderConnection: mockCreateProviderConnection,
}))

vi.mock('@moatless/api-client/sandbox/task-routes/task-routes', () => ({
  listTaskRoutes: mockListTaskRoutes,
  createTaskRoute: mockCreateTaskRoute,
  updateTaskRoute: mockUpdateTaskRoute,
}))

vi.mock('@moatless/api-client/sandbox/triggers/triggers', () => ({
  listTaskTriggers: mockListTaskTriggers,
  createTaskTrigger: mockCreateTaskTrigger,
  updateTaskTrigger: mockUpdateTaskTrigger,
  deleteTaskTrigger: mockDeleteTaskTrigger,
}))

describe('integrationsApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('uses generated provider and trigger handlers for trigger creation inputs', async () => {
    const connectionRequest: CreateProviderConnectionRequest = {
      provider: IntegrationProvider.slack,
      externalAccountId: 'T123',
      sourceName: 'Workspace',
      connectionKind: 'bot_api',
      webhookSecret: null,
      transportConfig: null,
    }
    const connectionResponse: ProviderConnectionResponse = {
      id: 'connection-1',
      provider: IntegrationProvider.slack,
      externalAccountId: 'T123',
      connectionKind: 'bot_api',
      createdBy: 'user-1',
      createdAt: '2026-05-07T10:00:00Z',
      updatedAt: '2026-05-07T10:00:00Z',
      scope: Scope.user,
    }
    const triggerRequest: CreateTaskTriggerRequest = {
      provider: IntegrationProvider.slack,
      connectionId: connectionResponse.id,
      sourceMatcher: 'C123',
      sourceName: '#triage',
      routeId: 'route-1',
      active: true,
      eventFilter: {},
      triggerConfig: { trigger: 'mention_only' },
    }
    const triggerResponse: TaskTriggerResponse = {
      id: 'trigger-1',
      provider: IntegrationProvider.slack,
      connectionId: connectionResponse.id,
      sourceMatcher: 'C123',
      sourceName: '#triage',
      routeId: 'route-1',
      active: true,
      eventFilter: {},
      triggerConfig: { trigger: 'mention_only' },
      allowDirectSend: false,
      closeTaskOnClose: true,
      notifyOnSuccess: false,
      createdBy: 'user-1',
      createdAt: '2026-05-07T10:00:00Z',
      updatedAt: '2026-05-07T11:00:00Z',
      scope: Scope.user,
    }

    mockCreateProviderConnection.mockResolvedValue(connectionResponse)
    mockCreateTaskTrigger.mockResolvedValue(triggerResponse)

    await expect(integrationsApi.createProviderConnection(connectionRequest)).resolves.toEqual(
      connectionResponse,
    )
    await expect(integrationsApi.createTaskTrigger(triggerRequest)).resolves.toEqual(
      triggerResponse,
    )

    expect(mockCreateProviderConnection).toHaveBeenCalledWith(connectionRequest)
    expect(mockCreateTaskTrigger).toHaveBeenCalledWith(triggerRequest)
  })

  it('uses generated task route handlers', async () => {
    const routeRequest = {
      repositoryId: 'repo-1',
      routingMode: RoutingMode.thread_based,
      promptTemplate: null,
      taskName: null,
      tagIds: [],
    }
    const routeResponse = {
      id: 'route-1',
      ...routeRequest,
      agentType: 'claude',
      createdBy: 'user-1',
      createdAt: '2026-05-07T10:00:00Z',
      updatedAt: '2026-05-07T10:00:00Z',
      scope: Scope.user,
    }

    mockCreateTaskRoute.mockResolvedValue(routeResponse)
    mockUpdateTaskRoute.mockResolvedValue(routeResponse)

    await expect(integrationsApi.createTaskRoute(routeRequest)).resolves.toEqual(routeResponse)
    await expect(integrationsApi.updateTaskRoute('route-1', routeRequest)).resolves.toEqual(
      routeResponse,
    )

    expect(mockCreateTaskRoute).toHaveBeenCalledWith(routeRequest)
    expect(mockUpdateTaskRoute).toHaveBeenCalledWith('route-1', routeRequest)
  })
})
