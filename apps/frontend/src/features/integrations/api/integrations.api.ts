import type {
  AdapterConnectionResponse,
  CreateAdapterConnectionRequest,
  CreateLoopRequest,
  Loop,
  LoopRecentTaskItem,
  UpdateLoopRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  createAdapterConnection as createAdapterConnectionHandler,
  listAdapterConnections as listAdapterConnectionsHandler,
  listAdapters as listAdaptersHandler,
} from '@moatless/api-client/sandbox/adapters/adapters'
import {
  createLoop as createTriggerHandler,
  deleteLoop as deleteTriggerHandler,
  getLoop as getTriggerHandler,
  listLoopRecentTasks as listTriggerRecentTasksHandler,
  listLoops as listTriggersHandler,
  updateLoop as updateTriggerHandler,
} from '@moatless/api-client/sandbox/loops/loops'

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

  async listTriggers(): Promise<Loop[]> {
    return await unwrapResponse<Loop[]>(listTriggersHandler())
  },

  async createTrigger(request: CreateLoopRequest): Promise<Loop> {
    return await unwrapResponse<Loop>(createTriggerHandler(request))
  },

  async updateTrigger(id: string, request: UpdateLoopRequest): Promise<Loop> {
    return await unwrapResponse<Loop>(updateTriggerHandler(id, request))
  },

  async deleteTrigger(id: string): Promise<void> {
    await unwrapResponse<void>(deleteTriggerHandler(id))
  },

  async getTrigger(id: string): Promise<Loop> {
    return await unwrapResponse<Loop>(getTriggerHandler(id))
  },

  async listTriggerRecentTasks(
    id: string,
    params?: { limit?: number },
  ): Promise<LoopRecentTaskItem[]> {
    return await unwrapResponse<LoopRecentTaskItem[]>(listTriggerRecentTasksHandler(id, params))
  },
}
