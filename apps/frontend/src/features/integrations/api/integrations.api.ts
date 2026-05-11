import type {
  AdapterConnectionResponse,
  CreateAdapterConnectionRequest,
  CreateTriggerRequest,
  TriggerRecentTaskItem,
  TriggerResponse,
  UpdateTriggerRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  createAdapterConnection as createAdapterConnectionHandler,
  listAdapterConnections as listAdapterConnectionsHandler,
  listAdapters as listAdaptersHandler,
} from '@moatless/api-client/sandbox/adapters/adapters'
import {
  createTrigger as createTriggerHandler,
  deleteTrigger as deleteTriggerHandler,
  getTrigger as getTriggerHandler,
  listTriggerRecentTasks as listTriggerRecentTasksHandler,
  listTriggers as listTriggersHandler,
  updateTrigger as updateTriggerHandler,
} from '@moatless/api-client/sandbox/triggers/triggers'

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

  async listTriggers(): Promise<TriggerResponse[]> {
    return await unwrapResponse<TriggerResponse[]>(listTriggersHandler())
  },

  async createTrigger(request: CreateTriggerRequest): Promise<TriggerResponse> {
    return await unwrapResponse<TriggerResponse>(createTriggerHandler(request))
  },

  async updateTrigger(id: string, request: UpdateTriggerRequest): Promise<TriggerResponse> {
    return await unwrapResponse<TriggerResponse>(updateTriggerHandler(id, request))
  },

  async deleteTrigger(id: string): Promise<void> {
    await unwrapResponse<void>(deleteTriggerHandler(id))
  },

  async getTrigger(id: string): Promise<TriggerResponse> {
    return await unwrapResponse<TriggerResponse>(getTriggerHandler(id))
  },

  async listTriggerRecentTasks(
    id: string,
    params?: { limit?: number },
  ): Promise<TriggerRecentTaskItem[]> {
    return await unwrapResponse<TriggerRecentTaskItem[]>(listTriggerRecentTasksHandler(id, params))
  },
}
