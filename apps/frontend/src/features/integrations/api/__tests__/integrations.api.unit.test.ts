import {
  type AdapterConnectionResponse,
  AdapterKind,
  type CreateAdapterConnectionRequest,
  Scope,
} from '@moatless/api-client'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { integrationsApi } from '../integrations.api'

const mockListAdapters = vi.hoisted(() => vi.fn())
const mockListAdapterConnections = vi.hoisted(() => vi.fn())
const mockCreateAdapterConnection = vi.hoisted(() => vi.fn())

vi.mock('@moatless/api-client/sandbox/adapters/adapters', () => ({
  listAdapters: mockListAdapters,
  listAdapterConnections: mockListAdapterConnections,
  createAdapterConnection: mockCreateAdapterConnection,
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
})
