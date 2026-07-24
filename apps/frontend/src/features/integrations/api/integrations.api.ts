import type {
  AdapterConnectionResponse,
  CreateAdapterConnectionRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  createAdapterConnection as createAdapterConnectionHandler,
  listAdapterConnections as listAdapterConnectionsHandler,
  listAdapters as listAdaptersHandler,
} from '@moatless/api-client/sandbox/adapters/adapters'

export const integrationsApi = {
  async listAdapters(): Promise<string[]> {
    return await unwrapResponse<string[]>(listAdaptersHandler())
  },

  async listAdapterConnections(): Promise<AdapterConnectionResponse[]> {
    return await unwrapResponse<AdapterConnectionResponse[]>(listAdapterConnectionsHandler())
  },

  async createAdapterConnection(
    request: CreateAdapterConnectionRequest,
  ): Promise<AdapterConnectionResponse> {
    return await unwrapResponse<AdapterConnectionResponse>(createAdapterConnectionHandler(request))
  },
}
